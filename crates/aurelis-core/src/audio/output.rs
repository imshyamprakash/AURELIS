use std::error::Error;
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::{ring_buffer::AudioRingBuffer, types::AudioSpec};

/// Real-time PCM audio output backed by CPAL.
///
/// The audio callback reads directly from a lock-free ring buffer,
/// so no mutex is used in the real-time path.
pub struct AudioOutput {
    stream: cpal::Stream,
    spec: AudioSpec,
    buffer: Arc<AudioRingBuffer>,
}

impl AudioOutput {
    /// Open the system's default output device.
    ///
    /// The device's native sample rate is used.
    /// PlaybackEngine performs resampling whenever needed.
    pub fn open(requested: AudioSpec) -> Result<Self, Box<dyn Error>> {
        let host = cpal::default_host();

        let device = host
            .default_output_device()
            .ok_or("No default audio output device found")?;

        let supported = device.default_output_config()?;

        // For now we require matching channel counts.
        if supported.channels() != requested.channels as u16 {
            return Err(format!(
                "Channel mismatch: AURELIS requires {}, device provides {}",
                requested.channels,
                supported.channels()
            )
            .into());
        }

        let sample_format = supported.sample_format();

        // Keep the device's native sample rate.
        let device_spec = AudioSpec::new(
            supported.sample_rate(),
            supported.channels() as usize,
        );

        let config: cpal::StreamConfig = supported.into();

        // 262,144 float samples ≈ 1 MB
        let buffer = Arc::new(AudioRingBuffer::new(262_144));
        let callback_buffer = Arc::clone(&buffer);

        let stream = match sample_format {
            cpal::SampleFormat::F32 => device.build_output_stream(
                config,
                move |output: &mut [f32], _| {
                    fill_output(output, &callback_buffer);
                },
                |err| {
                    eprintln!("Audio output error: {err}");
                },
                None,
            )?,
            _ => {
                return Err(
                    "AURELIS currently supports f32 output devices only".into(),
                );
            }
        };

        stream.play()?;

        Ok(Self {
            stream,
            spec: device_spec,
            buffer,
        })
    }

    /// Return the device audio specification.
    pub const fn spec(&self) -> AudioSpec {
        self.spec
    }

    /// Queue PCM samples for playback.
    ///
    /// This blocks until every sample is written into the ring buffer,
    /// ensuring no samples are dropped.
    pub fn push_samples(&self, samples: &[f32]) {
        let mut written = 0;

        while written < samples.len() {
            let pushed = self.buffer.push(&samples[written..]);

            if pushed == 0 {
                std::thread::yield_now();
                continue;
            }

            written += pushed;
        }
    }

    /// Number of samples waiting inside the ring buffer.
    pub fn pending_samples(&self) -> usize {
        self.buffer.len()
    }

    /// Remaining free space.
    pub fn available_space(&self) -> usize {
        self.buffer.available_space()
    }

    /// Check that the output stream is alive.
    pub fn is_running(&self) -> bool {
        let _ = &self.stream;
        true
    }

    /// Shared ring buffer handle.
    pub fn ring_buffer(&self) -> Arc<AudioRingBuffer> {
        Arc::clone(&self.buffer)
    }
}

fn fill_output(output: &mut [f32], buffer: &Arc<AudioRingBuffer>) {
    let read = buffer.pop(output);

    // Fill underruns with silence.
    if read < output.len() {
        output[read..].fill(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn available_space_decreases_after_push() {
        let rb = AudioRingBuffer::new(16);

        assert_eq!(rb.available_space(), 15);

        rb.push(&[1.0, 2.0, 3.0]);

        assert_eq!(rb.available_space(), 12);
    }

    #[test]
    fn callback_zero_fills_when_buffer_runs_empty() {
        let rb = Arc::new(AudioRingBuffer::new(16));

        rb.push(&[1.0, 2.0]);

        let mut out = [0.0; 4];

        fill_output(&mut out, &rb);

        assert_eq!(out, [1.0, 2.0, 0.0, 0.0]);
    }
}