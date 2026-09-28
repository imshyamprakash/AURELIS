use std::{
    error::Error,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use super::{
    AudioDecoder, AudioOutput, AudioResampler, AudioSpec,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
    Finished,
}

pub struct PlaybackEngine {
    source_spec: AudioSpec,
    output_spec: AudioSpec,
    resampling: bool,

    stop_flag: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl PlaybackEngine {
    /// Create a new playback session.
    pub fn open(path: &str) -> Result<Self, Box<dyn Error>> {
        let decoder = AudioDecoder::open(path)?;
        let source_spec = decoder.spec();

        let output = AudioOutput::open(source_spec)?;
        let output_spec = output.spec();

        let resampling = source_spec.sample_rate != output_spec.sample_rate;

        drop(decoder);
        drop(output);

        Ok(Self {
            source_spec,
            output_spec,
            resampling,
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

    /// Start playback on a dedicated worker thread.
    pub fn play(&mut self, path: String) -> Result<(), Box<dyn Error>> {
        if self.worker.is_some() {
            return Ok(());
        }

        self.stop_flag.store(false, Ordering::SeqCst);

        let stop = Arc::clone(&self.stop_flag);

        let handle = thread::spawn(move || {
            if let Err(err) = playback_worker(path, stop) {
                eprintln!("Playback failed: {err}");
            }
        });

        self.worker = Some(handle);

        Ok(())
    }

    /// Wait until playback finishes.
    pub fn wait(&mut self) {
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }

    /// Stop playback.
    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        self.wait();
    }
}

impl Drop for PlaybackEngine {
    fn drop(&mut self) {
        self.stop();
    }
}

fn playback_worker(
    path: String,
    stop: Arc<AtomicBool>,
) -> Result<(), Box<dyn Error>> {
    let mut decoder = AudioDecoder::open(&path)?;
    let source_spec = decoder.spec();

    let output = AudioOutput::open(source_spec)?;
    let output_spec = output.spec();

    let mut resampler = if source_spec.sample_rate != output_spec.sample_rate {
        Some(AudioResampler::new(source_spec, output_spec)?)
    } else {
        None
    };

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        let chunk = match decoder.next_chunk()? {
            Some(chunk) => chunk,
            None => break,
        };

        let is_last = chunk.frame_count() < decoder.chunk_frames();

        let pcm = if let Some(r) = resampler.as_mut() {
            if is_last {
                r.process_final(&chunk)?
            } else {
                r.process(&chunk)?
            }
        } else {
            chunk
        };

        if !pcm.samples.is_empty() {
            output.push_samples(&pcm.samples);
        }

        while output.pending_samples() > 65_536 {
            if stop.load(Ordering::Relaxed) {
                break;
            }

            thread::sleep(Duration::from_millis(2));
        }
    }

    if let Some(r) = resampler.as_mut() {
        let tail = r.flush()?;

        if !tail.samples.is_empty() {
            output.push_samples(&tail.samples);
        }
    }

    while output.pending_samples() > 0 {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        thread::sleep(Duration::from_millis(20));
    }

    Ok(())
}