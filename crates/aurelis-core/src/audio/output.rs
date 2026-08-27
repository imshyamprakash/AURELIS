use std::collections::VecDeque;
use std::error::Error;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::types::AudioSpec;

/// Real-time PCM audio output backed by CPAL.
pub struct AudioOutput {
    stream: cpal::Stream,
    spec: AudioSpec,
    buffer: Arc<Mutex<VecDeque<f32>>>,
}

impl AudioOutput {
    /// Open the system's default output device.
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

        let buffer = Arc::new(Mutex::new(VecDeque::<f32>::new()));
        let callback_buffer = Arc::clone(&buffer);

        let stream = match sample_format {
            cpal::SampleFormat::F32 => device.build_output_stream(
                config,
                move |output: &mut [f32], _| {
                    fill_output(output, &callback_buffer);
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
            buffer,
        })
    }

    /// Return the audio specification used by this output.
    pub const fn spec(&self) -> AudioSpec {
        self.spec
    }

    /// Queue PCM samples for playback.
    pub fn push_samples(&self, samples: &[f32]) {
        if let Ok(mut buffer) = self.buffer.lock() {
            buffer.extend(samples.iter().copied());
        }
    }

    /// Return the number of samples waiting for playback.
    pub fn pending_samples(&self) -> usize {
        self.buffer
            .lock()
            .map(|buffer| buffer.len())
            .unwrap_or(0)
    }

    /// Check that the output stream is alive.
    pub fn is_running(&self) -> bool {
        let _ = &self.stream;
        true
    }
}

fn fill_output(output: &mut [f32], buffer: &Arc<Mutex<VecDeque<f32>>>) {
    if let Ok(mut buffer) = buffer.lock() {
        for sample in output.iter_mut() {
            *sample = buffer.pop_front().unwrap_or(0.0);
        }
    } else {
        output.fill(0.0);
    }
}