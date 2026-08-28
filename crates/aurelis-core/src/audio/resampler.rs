use std::error::Error;

use rubato::{
    audioadapter_buffers::direct::{
        SequentialSliceOfSlices,
        SequentialSliceOfVecs,
    },
    Fft,
    FixedSync,
    Indexing,
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
///
/// The underlying resampler is configured with `FixedSync::Input`, which
/// means it always expects exactly [`AudioResampler::input_frames_next`]
/// frames per call to [`AudioResampler::process`]. The final chunk of a
/// stream is almost never that exact size, so it must go through
/// [`AudioResampler::process_final`] instead, followed by
/// [`AudioResampler::flush`] to drain the filter's internal delay line.
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

    /// Return the exact number of input frames a normal call to
    /// [`AudioResampler::process`] requires.
    ///
    /// Any decoder chunk with fewer frames than this (i.e. the final
    /// chunk of a stream) must be passed to
    /// [`AudioResampler::process_final`] instead.
    pub fn input_frames_next(&self) -> usize {
        self.resampler.input_frames_next()
    }

    /// Resample one *full-size* PCM buffer.
    ///
    /// `input` must contain exactly [`AudioResampler::input_frames_next`]
    /// frames. Use [`AudioResampler::process_final`] for the last, shorter
    /// chunk of a stream.
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
            return Ok(PcmBuffer::new(self.output_spec, Vec::new()));
        }

        let channels = self.input_spec.channels;
        self.validate_alignment(input, channels)?;

        let input_frames = input.frame_count();
        let input_channels = Self::deinterleave(input, channels);

        self.run(&input_channels, input_frames, None)
    }

    /// Resample the final, possibly short, chunk of a stream.
    ///
    /// Unlike [`AudioResampler::process`], `input` may contain fewer than
    /// [`AudioResampler::input_frames_next`] frames (this is normal: the
    /// last decoder chunk of a file is rarely a clean multiple of the
    /// chunk size). Rubato zero-pads the missing frames internally via
    /// [`Indexing::partial_len`]; AURELIS never manually pads or trims
    /// buffers.
    ///
    /// After calling this, call [`AudioResampler::flush`] once to release
    /// any real audio still held in the resampler's internal delay line.
    pub fn process_final(
        &mut self,
        input: &PcmBuffer,
    ) -> Result<PcmBuffer, Box<dyn Error>> {
        if input.spec != self.input_spec {
            return Err(
                "Input PCM specification does not match resampler".into(),
            );
        }

        let channels = self.input_spec.channels;
        let expected = self.input_frames_next();

        if input.samples.is_empty() {
            // Nothing real left to feed; treat this the same as flush.
            return self.run(&vec![Vec::new(); channels], 0, Some(0));
        }

        self.validate_alignment(input, channels)?;

        let input_frames = input.frame_count();

        if input_frames > expected {
            return Err(format!(
                "process_final received {input_frames} frames, but the \
                 final chunk must not exceed {expected} frames \
                 (use process() for full-size chunks)"
            )
            .into());
        }

        let input_channels = Self::deinterleave(input, channels);

        self.run(&input_channels, input_frames, Some(input_frames))
    }

    /// Drain the resampler's internal delay line after the stream has
    /// ended.
    ///
    /// The FFT-based resampler carries a small amount of filter state
    /// (overlap) between calls. After the true final chunk has been
    /// passed to [`AudioResampler::process_final`], some of the last
    /// real audio may still be sitting in that state rather than in the
    /// output already returned. This call pushes silence through the
    /// resampler once so that state is folded into real output instead
    /// of being lost. Call it exactly once per stream, after the final
    /// chunk, before dropping or resetting the resampler.
    pub fn flush(&mut self) -> Result<PcmBuffer, Box<dyn Error>> {
        let channels = self.output_spec.channels;
        self.run(&vec![Vec::new(); channels], 0, Some(0))
    }

    /// Reset the internal resampler state.
    pub fn reset(&mut self) {
        self.resampler.reset();
    }

    /// Shared plumbing for `process`, `process_final`, and `flush`.
    ///
    /// `input_channels` holds one `Vec<f32>` per channel, each at least
    /// `input_frames` long. `partial_len` mirrors
    /// [`Indexing::partial_len`]: `None` means "this is a full chunk of
    /// `input_frames_next()` frames", `Some(n)` means "only the first `n`
    /// frames are real, pad the rest with silence".
    fn run(
        &mut self,
        input_channels: &[Vec<f32>],
        input_frames: usize,
        partial_len: Option<usize>,
    ) -> Result<PcmBuffer, Box<dyn Error>> {
        let channels = self.input_spec.channels;

        // Rubato requires the input adapter to report at least as many
        // frames as it will actually read (`partial_len`, or the full
        // chunk size when `partial_len` is `None`). For a genuine partial
        // chunk that's exactly `input_frames`; for flush calls it's 0.
        let input_adapter = SequentialSliceOfVecs::new(
            input_channels,
            channels,
            input_frames,
        )?;

        let output_frames = self.resampler.output_frames_next();

        let mut output_channels =
            vec![vec![0.0f32; output_frames]; channels];

        let mut output_slices: Vec<&mut [f32]> = output_channels
            .iter_mut()
            .map(Vec::as_mut_slice)
            .collect();

        let mut output_adapter = SequentialSliceOfSlices::new_mut(
            output_slices.as_mut_slice(),
            channels,
            output_frames,
        )?;

        let indexing = partial_len.map(|len| Indexing::new().partial_len(len));

        let (_, output_frames_produced) = self.resampler.process_into_buffer(
            &input_adapter,
            &mut output_adapter,
            indexing.as_ref(),
        )?;

        let mut interleaved =
            Vec::with_capacity(output_frames_produced * channels);

        for frame in 0..output_frames_produced {
            for channel in output_channels.iter() {
                interleaved.push(channel[frame]);
            }
        }

        Ok(PcmBuffer::new(self.output_spec, interleaved))
    }

    fn validate_alignment(
        &self,
        input: &PcmBuffer,
        channels: usize,
    ) -> Result<(), Box<dyn Error>> {
        if input.samples.len() % channels != 0 {
            return Err(
                "PCM sample count is not aligned to channel count".into(),
            );
        }

        Ok(())
    }

    fn deinterleave(input: &PcmBuffer, channels: usize) -> Vec<Vec<f32>> {
        let input_frames = input.frame_count();
        let mut input_channels =
            vec![Vec::<f32>::with_capacity(input_frames); channels];

        for frame in input.samples.chunks_exact(channels) {
            for (channel, &sample) in frame.iter().enumerate() {
                input_channels[channel].push(sample);
            }
        }

        input_channels
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

        let mut resampler = AudioResampler::new(input, output).unwrap();

        let buffer = test_buffer(input, resampler.input_frames_next());
        let result = resampler.process(&buffer).unwrap();

        assert_eq!(result.spec, output);
        assert!(!result.samples.is_empty());
    }

    #[test]
    fn resampler_accepts_88200_to_48000() {
        let input = AudioSpec::new(88_200, 2);
        let output = AudioSpec::new(48_000, 2);

        let mut resampler = AudioResampler::new(input, output).unwrap();

        let buffer = test_buffer(input, resampler.input_frames_next());
        let result = resampler.process(&buffer).unwrap();

        assert_eq!(result.spec, output);
        assert!(!result.samples.is_empty());
    }

    #[test]
    fn resampler_accepts_96000_to_48000() {
        let input = AudioSpec::new(96_000, 2);
        let output = AudioSpec::new(48_000, 2);

        let mut resampler = AudioResampler::new(input, output).unwrap();

        let buffer = test_buffer(input, resampler.input_frames_next());
        let result = resampler.process(&buffer).unwrap();

        assert_eq!(result.spec, output);
        assert!(!result.samples.is_empty());
    }

    #[test]
    fn resampler_accepts_192000_to_48000() {
        let input = AudioSpec::new(192_000, 2);
        let output = AudioSpec::new(48_000, 2);

        let mut resampler = AudioResampler::new(input, output).unwrap();

        let buffer = test_buffer(input, resampler.input_frames_next());
        let result = resampler.process(&buffer).unwrap();

        assert_eq!(result.spec, output);
        assert!(!result.samples.is_empty());
    }

    #[test]
    fn resampler_rejects_channel_conversion() {
        let input = AudioSpec::new(96_000, 2);
        let output = AudioSpec::new(48_000, 1);

        assert!(AudioResampler::new(input, output).is_err());
    }

    #[test]
    fn resampler_rejects_identical_sample_rate() {
        let input = AudioSpec::new(48_000, 2);
        let output = AudioSpec::new(48_000, 2);

        assert!(AudioResampler::new(input, output).is_err());
    }

    #[test]
    fn resampler_rejects_mismatched_input_spec() {
        let input = AudioSpec::new(96_000, 2);
        let output = AudioSpec::new(48_000, 2);
        let wrong = AudioSpec::new(44_100, 2);

        let mut resampler = AudioResampler::new(input, output).unwrap();

        let buffer = test_buffer(wrong, resampler.input_frames_next());

        assert!(resampler.process(&buffer).is_err());
    }

    /// Reproduces the exact failure from the current AURELIS bug report:
    /// a 96 kHz -> 48 kHz stream whose final decoder chunk is far shorter
    /// than the resampler's configured chunk size (47 frames vs 4096).
    #[test]
    fn process_final_handles_short_trailing_chunk() {
        let input = AudioSpec::new(96_000, 2);
        let output = AudioSpec::new(48_000, 2);

        let mut resampler = AudioResampler::new(input, output).unwrap();

        // A few normal full-size chunks first, like real playback.
        let full = test_buffer(input, resampler.input_frames_next());
        for _ in 0..3 {
            resampler.process(&full).unwrap();
        }

        // Then the short final chunk that previously crashed the pipeline.
        let tail = test_buffer(input, 47);
        let result = resampler.process_final(&tail);

        assert!(
            result.is_ok(),
            "process_final should accept a short final chunk, got {:?}",
            result.err()
        );

        let result = result.unwrap();
        assert_eq!(result.spec, output);
    }

    #[test]
    fn process_final_rejects_oversized_chunk() {
        let input = AudioSpec::new(96_000, 2);
        let output = AudioSpec::new(48_000, 2);

        let mut resampler = AudioResampler::new(input, output).unwrap();

        let too_big =
            test_buffer(input, resampler.input_frames_next() + 1);

        assert!(resampler.process_final(&too_big).is_err());
    }

    #[test]
    fn flush_drains_without_error_after_final_chunk() {
        let input = AudioSpec::new(96_000, 2);
        let output = AudioSpec::new(48_000, 2);

        let mut resampler = AudioResampler::new(input, output).unwrap();

        let tail = test_buffer(input, 47);
        resampler.process_final(&tail).unwrap();

        let flushed = resampler.flush();

        assert!(
            flushed.is_ok(),
            "flush should never error, got {:?}",
            flushed.err()
        );
        assert_eq!(flushed.unwrap().spec, output);
    }

    #[test]
    fn flush_is_safe_with_no_prior_processing() {
        let input = AudioSpec::new(96_000, 2);
        let output = AudioSpec::new(48_000, 2);

        let mut resampler = AudioResampler::new(input, output).unwrap();

        assert!(resampler.flush().is_ok());
    }
}