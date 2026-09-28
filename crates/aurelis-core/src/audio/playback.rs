use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::{
    AudioDecoder,
    AudioOutput,
    AudioQueue,
    AudioResampler,
    AudioSpec,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Stopped,
    Playing,
}

pub struct PlaybackEngine {
    source_spec: AudioSpec,
    output_spec: AudioSpec,
    resampling: bool,

    queue: AudioQueue,

    stop_flag: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl PlaybackEngine {
    pub fn open(path: &str) -> Result<Self, Box<dyn Error>> {
        let decoder = AudioDecoder::open(path)?;
        let source_spec = decoder.spec();

        let output = AudioOutput::open(source_spec)?;
        let output_spec = output.spec();

        Ok(Self {
            source_spec,
            output_spec,
            resampling: source_spec.sample_rate != output_spec.sample_rate,
            queue: AudioQueue::new(),
            stop_flag: Arc::new(AtomicBool::new(false)),
            worker: None,
        })
    }

    pub fn source_spec(&self) -> AudioSpec {
        self.source_spec
    }

    pub fn output_spec(&self) -> AudioSpec {
        self.output_spec
    }

    pub fn is_resampling(&self) -> bool {
        self.resampling
    }

    pub fn state(&self) -> PlaybackState {
        if self.worker.is_some() {
            PlaybackState::Playing
        } else {
            PlaybackState::Stopped
        }
    }

    // ---------------- Queue ----------------

    pub fn enqueue<P: AsRef<Path>>(&mut self, path: P) {
        self.queue.push(path);
    }

    pub fn next_track(&mut self) -> Option<PathBuf> {
        self.queue.pop()
    }

    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    pub fn queue_is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn peek_track(&self) -> Option<&Path> {
        self.queue.peek()
    }

    // ---------------- Playback ----------------

    pub fn play(&mut self, path: String) -> Result<(), Box<dyn Error>> {
        if self.worker.is_some() {
            return Ok(());
        }

        self.stop_flag.store(false, Ordering::Relaxed);

        let stop_flag = Arc::clone(&self.stop_flag);
        let output_spec = self.output_spec;

        let handle = thread::spawn(move || {
            if let Err(error) =
                playback_worker(&path, output_spec, stop_flag)
            {
                eprintln!("Playback failed: {error}");
            }
        });

        self.worker = Some(handle);

        Ok(())
    }

    pub fn wait(&mut self) {
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }

    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        self.wait();
    }
}

impl Drop for PlaybackEngine {
    fn drop(&mut self) {
        self.stop();
    }
}

fn playback_worker(
    path: &str,
    output_spec: AudioSpec,
    stop_flag: Arc<AtomicBool>,
) -> Result<(), Box<dyn Error>> {
    let mut decoder = AudioDecoder::open(path)?;
    let source_spec = decoder.spec();

    let output = AudioOutput::open(output_spec)?;

    let mut resampler = if source_spec.sample_rate != output_spec.sample_rate {
        Some(AudioResampler::new(source_spec, output_spec)?)
    } else {
        None
    };

    while !stop_flag.load(Ordering::Relaxed) {
        let chunk = match decoder.next_chunk()? {
            Some(chunk) => chunk,
            None => break,
        };

        let is_final_chunk = decoder.is_finished();

        if let Some(resampler) = &mut resampler {
            let converted = if is_final_chunk {
                resampler.process_final(&chunk)?
            } else {
                resampler.process(&chunk)?
            };

            output.push_samples(&converted.samples);
        } else {
            output.push_samples(&chunk.samples);
        }
    }

    if let Some(resampler) = &mut resampler {
        let remaining = resampler.flush()?;

        if !remaining.samples.is_empty() {
            output.push_samples(&remaining.samples);
        }
    }

    while output.pending_samples() > 0 {
        thread::sleep(Duration::from_millis(10));
    }

    Ok(())
}