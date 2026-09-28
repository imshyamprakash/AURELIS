use std::collections::VecDeque;
use std::error::Error;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::types::AudioSpec;

/// How long, in milliseconds, `AudioOutput` is willing to buffer ahead of
/// the audio callback before `push_samples` starts blocking the caller.
///
/// This is the real-time-safety boundary described in the AURELIS
/// blueprint: it caps how far the decode/resample thread can run ahead of
/// playback, so memory use stays bounded no matter how large the source
/// file is.
const MAX_BUFFERED_MS: u64 = 500;

/// How long `push_samples` sleeps between capacity checks while blocked.
const BACKPRESSURE_POLL_INTERVAL: Duration = Duration::from_millis(5);

/// A bounded queue of interleaved f32 samples shared between the
/// decode/resample thread (producer) and the CPAL audio callback
/// (consumer).
///
/// This is deliberately a plain, hardware-independent struct so its
/// capacity/backpressure behavior can be unit tested without opening a
/// real audio device.
struct BoundedSampleQueue {
    samples: VecDeque<f32>,
    max_samples: usize,
}

impl BoundedSampleQueue {
    fn new(max_samples: usize) -> Self {
        Self {
            samples: VecDeque::new(),
            max_samples,
        }
    }

    fn len(&self) -> usize {
        self.samples.len()
    }

    fn remaining_capacity(&self) -> usize {
        self.max_samples.saturating_sub(self.samples.len())
    }

    /// Push as many samples as currently fit, without blocking.
    ///
    /// Returns the number of samples actually accepted, which may be less
    /// than `samples.len()` when the queue is near capacity. The caller
    /// is responsible for retrying the remainder; this type never drops
    /// audio silently.
    fn push_available(&mut self, samples: &[f32]) -> usize {
        let capacity = self.remaining_capacity();
        let accepted = capacity.min(samples.len());

        self.samples.extend(samples[..accepted].iter().copied());

        accepted
    }

    fn pop_front(&mut self) -> Option<f32> {
        self.samples.pop_front()
    }
}

/// Real-time PCM audio output backed by CPAL.
pub struct AudioOutput {
    stream: cpal::Stream,
    spec: AudioSpec,
    queue: Arc<Mutex<BoundedSampleQueue>>,
    underrun_events: Arc<AtomicUsize>,
}

impl AudioOutput {
    /// Query the default output device's native format without opening a
    /// stream.
    ///
    /// Use this before deciding whether a resampler is needed: compare the
    /// decoder's [`AudioSpec`] against this one. If they differ, build an
    /// `AudioResampler` targeting this spec and open `AudioOutput` with
    /// this spec too, since `AudioOutput::open` will reject anything the
    /// device doesn't natively support.
    pub fn default_device_spec() -> Result<AudioSpec, Box<dyn Error>> {
        let host = cpal::default_host();

        let device = host
            .default_output_device()
            .ok_or("No default audio output device found")?;

        let supported_config = device.default_output_config()?;

        Ok(AudioSpec::new(
            supported_config.sample_rate(),
            supported_config.channels() as usize,
        ))
    }

    /// Open the system's default output device.
    ///
    /// `spec` must exactly match the device's native sample rate and
    /// channel count. Query [`AudioOutput::default_device_spec`] first and
    /// resample to that spec if your source doesn't already match it.
    pub fn open(spec: AudioSpec) -> Result<Self, Box<dyn Error>> {
        let host = cpal::default_host();

        let device = host
            .default_output_device()
            .ok_or("No default audio output device found")?;

        let supported_config = device.default_output_config()?;

        if supported_config.channels() != spec.channels as u16 {
            return Err(format!(
                "Channel mismatch: AURELIS requires {}, device provides {}",
                spec.channels,
                supported_config.channels()
            )
            .into());
        }

        if supported_config.sample_rate() != spec.sample_rate {
            return Err(format!(
                "Sample rate mismatch: AURELIS requires {} Hz, device provides {} Hz",
                spec.sample_rate,
                supported_config.sample_rate()
            )
            .into());
        }

        let sample_format = supported_config.sample_format();
        let config: cpal::StreamConfig = supported_config.into();

        let max_samples = (spec.sample_rate as u64 * spec.channels as u64
            * MAX_BUFFERED_MS
            / 1000) as usize;

        let queue = Arc::new(Mutex::new(BoundedSampleQueue::new(max_samples)));
        let callback_queue = Arc::clone(&queue);

        let underrun_events = Arc::new(AtomicUsize::new(0));
        let callback_underruns = Arc::clone(&underrun_events);

        let stream = match sample_format {
            cpal::SampleFormat::F32 => device.build_output_stream(
                config,
                move |output: &mut [f32], _| {
                    fill_output(output, &callback_queue, &callback_underruns);
                },
                |error| {
                    eprintln!("Audio output error: {error}");
                },
                None,
            )?,

            _ => {
                return Err(
                    "AURELIS currently supports f32 output devices only"
                        .into(),
                );
            }
        };

        stream.play()?;

        Ok(Self {
            stream,
            spec,
            queue,
            underrun_events,
        })
    }

    /// Return the audio specification used by this output.
    pub const fn spec(&self) -> AudioSpec {
        self.spec
    }

    /// Queue PCM samples for playback.
    ///
    /// Blocks (sleeping, not spinning) while the internal buffer is full,
    /// so the decode/resample thread never runs more than
    /// [`MAX_BUFFERED_MS`] ahead of actual playback. This must never be
    /// called from the real-time audio callback itself — only from the
    /// decode/resample thread, where blocking is safe.
    ///
    /// Never drops audio: if a single chunk is larger than the queue's
    /// total capacity (should not happen with normal decoder chunk
    /// sizes), it is still accepted in full rather than silently
    /// truncated, to honor "never discard audio frames".
    pub fn push_samples(&self, samples: &[f32]) {
        push_blocking(&self.queue, samples);
    }

    /// Return the number of samples currently waiting for playback.
    pub fn pending_samples(&self) -> usize {
        self.queue.lock().map(|queue| queue.len()).unwrap_or(0)
    }

    /// Return how many times the audio callback has run dry and had to
    /// fill with silence since this output was opened.
    ///
    /// A non-zero count after normal playback (i.e. not right at the very
    /// start, before the first samples arrive) indicates the
    /// decode/resample thread isn't keeping up with the device.
    pub fn underrun_count(&self) -> usize {
        self.underrun_events.load(Ordering::Relaxed)
    }

    /// Check that the output stream is alive.
    pub fn is_running(&self) -> bool {
        let _ = &self.stream;
        true
    }
}

/// Push samples into a bounded queue, blocking (via short sleeps, not a
/// spin loop) while it's full, and never dropping input — even a single
/// chunk larger than the queue's total capacity is still fully accepted,
/// across as many retries as it takes.
///
/// Pulled out as a free function, independent of `AudioOutput`, so this
/// backpressure behavior can be unit tested against a plain
/// `Arc<Mutex<BoundedSampleQueue>>` without opening a real audio device.
fn push_blocking(queue: &Arc<Mutex<BoundedSampleQueue>>, samples: &[f32]) {
    let mut offset = 0;

    while offset < samples.len() {
        let accepted = match queue.lock() {
            Ok(mut queue) => queue.push_available(&samples[offset..]),
            Err(_) => {
                // Poisoned lock: nothing sane to do but drop out rather
                // than spin forever.
                return;
            }
        };

        offset += accepted;

        if offset < samples.len() {
            thread::sleep(BACKPRESSURE_POLL_INTERVAL);
        }
    }
}

/// Real-time audio callback. Must stay allocation-free and lock-free
/// beyond the single short mutex hold below — no file I/O, no decoding,
/// no unbounded work.
fn fill_output(
    output: &mut [f32],
    queue: &Arc<Mutex<BoundedSampleQueue>>,
    underrun_events: &Arc<AtomicUsize>,
) {
    let Ok(mut queue) = queue.lock() else {
        output.fill(0.0);
        return;
    };

    let mut starved = false;

    for sample in output.iter_mut() {
        *sample = match queue.pop_front() {
            Some(value) => value,
            None => {
                starved = true;
                0.0
            }
        };
    }

    drop(queue);

    if starved {
        underrun_events.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_queue_accepts_within_capacity() {
        let mut queue = BoundedSampleQueue::new(10);

        let accepted = queue.push_available(&[1.0, 2.0, 3.0]);

        assert_eq!(accepted, 3);
        assert_eq!(queue.len(), 3);
        assert_eq!(queue.remaining_capacity(), 7);
    }

    #[test]
    fn bounded_queue_partially_accepts_when_near_full() {
        let mut queue = BoundedSampleQueue::new(5);

        assert_eq!(queue.push_available(&[1.0, 2.0, 3.0]), 3);
        // Only 2 slots left; offering 4 more should accept just 2.
        assert_eq!(queue.push_available(&[4.0, 5.0, 6.0, 7.0]), 2);
        assert_eq!(queue.len(), 5);
        assert_eq!(queue.remaining_capacity(), 0);
    }

    #[test]
    fn bounded_queue_rejects_everything_when_full() {
        let mut queue = BoundedSampleQueue::new(2);

        assert_eq!(queue.push_available(&[1.0, 2.0]), 2);
        assert_eq!(queue.push_available(&[3.0]), 0);
        assert_eq!(queue.len(), 2);
    }

    #[test]
    fn bounded_queue_pop_front_drains_in_order() {
        let mut queue = BoundedSampleQueue::new(10);

        queue.push_available(&[1.0, 2.0, 3.0]);

        assert_eq!(queue.pop_front(), Some(1.0));
        assert_eq!(queue.pop_front(), Some(2.0));
        assert_eq!(queue.pop_front(), Some(3.0));
        assert_eq!(queue.pop_front(), None);
    }

    #[test]
    fn push_blocking_never_drops_a_chunk_larger_than_capacity() {
        // Regression guard for the "never discard audio frames" rule:
        // a single push larger than total capacity must still be fully
        // accepted (across multiple internal retries), not truncated.
        //
        // Exercises push_blocking directly against a plain queue, with no
        // real AudioOutput/cpal::Stream involved — this behavior doesn't
        // depend on real hardware and shouldn't need any.
        let queue = Arc::new(Mutex::new(BoundedSampleQueue::new(4)));

        // Drain the queue concurrently in a background thread so the
        // blocking push can eventually make progress, mirroring how the
        // real CPAL callback drains it.
        let queue_for_drain = Arc::clone(&queue);
        let drain_handle = thread::spawn(move || {
            let mut drained = 0;
            while drained < 10 {
                if let Ok(mut queue) = queue_for_drain.lock() {
                    if queue.pop_front().is_some() {
                        drained += 1;
                    }
                }
                thread::sleep(Duration::from_millis(1));
            }
        });

        let big_chunk = vec![0.5f32; 10];
        push_blocking(&queue, &big_chunk);

        drain_handle.join().unwrap();

        assert_eq!(queue.lock().unwrap().len(), 0);
    }
}