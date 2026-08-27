use std::error::Error;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::types::AudioSpec;

/// Basic CPAL audio output device.
///
/// This is the first layer between AURELIS's PCM pipeline
/// and the system audio device.
pub struct AudioOutput {
    stream: cpal::Stream,
    spec: AudioSpec,
    buffer: Arc<Mutex<Vec<f32>>>,
}

impl AudioOutput {
    /// Open the system's default output device.
    pub fn open(spec: AudioSpec) -> Result<Self, Box<dyn Error>> {
        let host = cpal::default_host();

        let device = host
            .default_output_device()
            .ok_or("No default audio output device found")?;

        let supported_config = device.default_output_config()?;

        let sample_format = supported_config.sample_format();
        let config: cpal::StreamConfig = supported_config.into();

        let buffer = Arc::new(Mutex::new(Vec::<f32>::new()));
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

    /// Return the audio specification requested by AURELIS.
    pub const fn spec(&self) -> AudioSpec {
        self.spec
    }

    /// Queue PCM samples for playback.
    pub fn push_samples(&self, samples: &[f32]) {
        if let Ok(mut buffer) = self.buffer.lock() {
            buffer.extend_from_slice(samples);
        }
    }

    /// Return the number of samples waiting for playback.
    pub fn pending_samples(&self) -> usize {
        self.buffer
            .lock()
            .map(|buffer| buffer.len())
            .unwrap_or(0)
    }

    /// Keep the CPAL stream alive.
    pub fn is_running(&self) -> bool {
        let _ = &self.stream;
        true
    }
}

fn fill_output(output: &mut [f32], buffer: &Arc<Mutex<Vec<f32>>>) {
    if let Ok(mut buffer) = buffer.lock() {
        let samples_to_copy = output.len().min(buffer.len());

        output[..samples_to_copy]
            .copy_from_slice(&buffer[..samples_to_copy]);

        if samples_to_copy < output.len() {
            output[samples_to_copy..].fill(0.0);
        }

        buffer.drain(..samples_to_copy);
    } else {
        output.fill(0.0);
    }
}