use std::error::Error;

use rubato::{
    audioadapter_buffers::direct::{
        SequentialSliceOfSlices,
        SequentialSliceOfVecs,
    },
    Fft,
    FixedSync,
    Resampler,
};

use super::types::{AudioSpec, PcmBuffer};

/// High-quality sample-rate converter for AURELIS.
///
/// The decoder preserves the source sample rate. This component converts
/// decoded PCM to the sample rate required by the output device.
///
/// Common source rates supported:
/// - 44,100 Hz
/// - 48,000 Hz
/// - 88,200 Hz
/// - 96,000 Hz
/// - 192,000 Hz
pub struct AudioResampler {
    input_spec: AudioSpec,
    output_spec: AudioSpec,
    resampler: Fft<f32>,
}

impl AudioResampler {
    /// Create a new sample-rate converter.
    ///
    /// Channel conversion is intentionally not performed here.
    /// Input and output must have the same number of channels.
    pub fn new(
        input_spec: AudioSpec,
        output_spec: AudioSpec,
    ) -> Result<Self, Box<dyn Error>> {
        if input_spec.sample_rate == 0 || output_spec.sample_rate == 0 {
            return Err("Sample rate must be greater than zero".into());
        }

        if input_spec.channels == 0 || output_spec.channels == 0 {
            return Err("Channel count must be greater than zero".into());
        }

        if input_spec.channels != output_spec.channels {
            return Err(format!(
                "Channel conversion is not supported: {} -> {}",
                input_spec.channels,
                output_spec.channels
            )
            .into());
        }

        if input_spec.sample_rate == output_spec.sample_rate {
            return Err(
                "Resampler is unnecessary when sample rates are identical"
                    .into(),
            );
        }

        let resampler = Fft::<f32>::new(
            input_spec.sample_rate as usize,
            output_spec.sample_rate as usize,
            4096,
            input_spec.channels,
            FixedSync::Input,
        )?;

        Ok(Self {
            input_spec,
            output_spec,
            resampler,
        })
    }

    /// Return the input specification.
    pub const fn input_spec(&self) -> AudioSpec {
        self.input_spec
    }

    /// Return the output specification.
    pub const fn output_spec(&self) -> AudioSpec {
        self.output_spec
    }

    /// Resample one PCM buffer.
    pub fn process(
        &mut self,
        input: &PcmBuffer,
    ) -> Result<PcmBuffer, Box<dyn Error>> {
        if input.spec != self.input_spec {
            return Err(
                "Input PCM specification does not match resampler".into(),
            );
        }

        if input.samples.is_empty() {
            return Ok(PcmBuffer::new(
                self.output_spec,
                Vec::new(),
            ));
        }

        let channels = self.input_spec.channels;

        if input.samples.len() % channels != 0 {
            return Err(
                "PCM sample count is not aligned to channel count".into(),
            );
        }

        let input_frames = input.frame_count();

        // Convert interleaved PCM into planar PCM.
        let mut input_channels =
            vec![Vec::<f32>::with_capacity(input_frames); channels];

        for frame in input.samples.chunks_exact(channels) {
            for channel in 0..channels {
                input_channels[channel].push(frame[channel]);
            }
        }

        // Read-only input adapter.
        let input_adapter = SequentialSliceOfVecs::new(
            &input_channels,
            channels,
            input_frames,
        )?;

        // Allocate maximum possible output.
        let output_frames = self.resampler.output_frames_max();

        let mut output_channels =
            vec![vec![0.0f32; output_frames]; channels];

        // Build mutable channel slices.
        let mut output_slices: Vec<&mut [f32]> = output_channels
            .iter_mut()
            .map(Vec::as_mut_slice)
            .collect();

        // Rubato 5 requires `new_mut` for mutable output.
        let mut output_adapter =
            SequentialSliceOfSlices::new_mut(
                output_slices.as_mut_slice(),
                channels,
                output_frames,
            )?;

        let (_, output_frames_produced) =
            self.resampler.process_into_buffer(
                &input_adapter,
                &mut output_adapter,
                None,
            )?;

        // Convert planar output back to interleaved PCM.
        let mut interleaved =
            Vec::with_capacity(output_frames_produced * channels);

        for frame in 0..output_frames_produced {
            for channel in 0..channels {
                interleaved.push(output_channels[channel][frame]);
            }
        }

        Ok(PcmBuffer::new(
            self.output_spec,
            interleaved,
        ))
    }

    /// Flush the resampler's internal state.
    pub fn process_partial(
        &mut self,
    ) -> Result<PcmBuffer, Box<dyn Error>> {
        let channels = self.output_spec.channels;

        let input_channels =
            vec![Vec::<f32>::new(); channels];

        let input_adapter =
            SequentialSliceOfVecs::new(
                &input_channels,
                channels,
                0,
            )?;

        let output_frames =
            self.resampler.output_frames_max();

        let mut output_channels =
            vec![vec![0.0f32; output_frames]; channels];

        let mut output_slices: Vec<&mut [f32]> =
            output_channels
                .iter_mut()
                .map(Vec::as_mut_slice)
                .collect();

        let mut output_adapter =
            SequentialSliceOfSlices::new_mut(
                output_slices.as_mut_slice(),
                channels,
                output_frames,
            )?;

        let (_, output_frames_produced) =
            self.resampler.process_into_buffer(
                &input_adapter,
                &mut output_adapter,
                None,
            )?;

        let mut interleaved =
            Vec::with_capacity(output_frames_produced * channels);

        for frame in 0..output_frames_produced {
            for channel in 0..channels {
                interleaved.push(output_channels[channel][frame]);
            }
        }

        Ok(PcmBuffer::new(
            self.output_spec,
            interleaved,
        ))
    }

    /// Reset the internal resampler state.
    pub fn reset(&mut self) {
        self.resampler.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::types::AudioSpec;

    fn test_buffer(spec: AudioSpec, frames: usize) -> PcmBuffer {
        let samples = vec![0.0f32; frames * spec.channels];

        PcmBuffer::new(spec, samples)
    }

    #[test]
    fn resampler_accepts_44100_to_48000() {
        let input = AudioSpec::new(44_100, 2);
        let output = AudioSpec::new(48_000, 2);

        let mut resampler =
            AudioResampler::new(input, output).unwrap();

        let buffer = test_buffer(input, 4096);
        let result = resampler.process(&buffer).unwrap();

        assert_eq!(result.spec, output);
        assert!(!result.samples.is_empty());
    }

    #[test]
    fn resampler_accepts_88200_to_48000() {
        let input = AudioSpec::new(88_200, 2);
        let output = AudioSpec::new(48_000, 2);

        let mut resampler =
            AudioResampler::new(input, output).unwrap();

        let buffer = test_buffer(input, 4096);
        let result = resampler.process(&buffer).unwrap();

        assert_eq!(result.spec, output);
        assert!(!result.samples.is_empty());
    }

    #[test]
    fn resampler_accepts_96000_to_48000() {
        let input = AudioSpec::new(96_000, 2);
        let output = AudioSpec::new(48_000, 2);

        let mut resampler =
            AudioResampler::new(input, output).unwrap();

        let buffer = test_buffer(input, 4096);
        let result = resampler.process(&buffer).unwrap();

        assert_eq!(result.spec, output);
        assert!(!result.samples.is_empty());
    }

    #[test]
    fn resampler_accepts_192000_to_48000() {
        let input = AudioSpec::new(192_000, 2);
        let output = AudioSpec::new(48_000, 2);

        let mut resampler =
            AudioResampler::new(input, output).unwrap();

        let buffer = test_buffer(input, 4096);
        let result = resampler.process(&buffer).unwrap();

        assert_eq!(result.spec, output);
        assert!(!result.samples.is_empty());
    }

    #[test]
    fn resampler_rejects_channel_conversion() {
        let input = AudioSpec::new(96_000, 2);
        let output = AudioSpec::new(48_000, 1);

        assert!(
            AudioResampler::new(input, output).is_err()
        );
    }

    #[test]
    fn resampler_rejects_identical_sample_rate() {
        let input = AudioSpec::new(48_000, 2);
        let output = AudioSpec::new(48_000, 2);

        assert!(
            AudioResampler::new(input, output).is_err()
        );
    }

    #[test]
    fn resampler_rejects_mismatched_input_spec() {
        let input = AudioSpec::new(96_000, 2);
        let output = AudioSpec::new(48_000, 2);
        let wrong = AudioSpec::new(44_100, 2);

        let mut resampler =
            AudioResampler::new(input, output).unwrap();

        let buffer = test_buffer(wrong, 4096);

        assert!(resampler.process(&buffer).is_err());
    }
}