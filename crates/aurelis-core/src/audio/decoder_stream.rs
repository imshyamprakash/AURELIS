use std::collections::VecDeque;
use std::fs::File;
use std::path::{Path, PathBuf};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::{FormatReader, FormatOptions};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::default::{get_codecs, get_probe};

use super::stream::DEFAULT_CHUNK_FRAMES;
use super::types::{AudioSpec, PcmBuffer};

/// A streaming audio decoder.
///
/// Audio packets are decoded progressively and converted into small
/// PCM chunks. Any samples that do not fit into the current chunk are
/// retained and returned by the next call.
pub struct AudioDecoder {
    path: PathBuf,
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    spec: AudioSpec,
    chunk_frames: usize,
    pending_samples: VecDeque<f32>,
    finished: bool,
}

impl AudioDecoder {
    /// Open an audio file using the default chunk size.
    pub fn open<P: AsRef<Path>>(
        path: P,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Self::open_with_chunk_size(path, DEFAULT_CHUNK_FRAMES)
    }

    /// Open an audio file with a custom PCM chunk size.
    pub fn open_with_chunk_size<P: AsRef<Path>>(
        path: P,
        chunk_frames: usize,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        if chunk_frames == 0 {
            return Err("Chunk size must be greater than zero".into());
        }

        let path = path.as_ref().to_path_buf();

        let file = File::open(&path)?;

        let media_source =
            MediaSourceStream::new(Box::new(file), Default::default());

        let hint = Hint::new();

        let probed = get_probe().format(
            &hint,
            media_source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )?;

        let format = probed.format;

        let track = format
            .default_track()
            .ok_or("No audio track found")?;

        let track_id = track.id;
        let codec_params = track.codec_params.clone();

        let sample_rate = codec_params
            .sample_rate
            .ok_or("Missing sample rate")?;

        let channels = codec_params
            .channels
            .ok_or("Missing channel information")?
            .count();

        let spec = AudioSpec::new(sample_rate, channels);

        let decoder =
            get_codecs().make(&codec_params, &DecoderOptions::default())?;

        Ok(Self {
            path,
            format,
            decoder,
            track_id,
            spec,
            chunk_frames,
            pending_samples: VecDeque::new(),
            finished: false,
        })
    }

    /// Return the source file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Return the audio specification.
    pub const fn spec(&self) -> AudioSpec {
        self.spec
    }

    /// Return the configured chunk size in frames.
    pub const fn chunk_frames(&self) -> usize {
        self.chunk_frames
    }

    /// Return whether the decoder has reached the end of the file.
    pub const fn is_finished(&self) -> bool {
        self.finished
    }

    /// Return the number of samples currently waiting to be emitted.
    pub fn pending_samples(&self) -> usize {
        self.pending_samples.len()
    }

    /// Decode and return the next PCM chunk.
    ///
    /// Samples that cross a chunk boundary are retained internally
    /// instead of being discarded.
    pub fn next_chunk(
        &mut self,
    ) -> Result<Option<PcmBuffer>, Box<dyn std::error::Error>> {
        if self.finished && self.pending_samples.is_empty() {
            return Ok(None);
        }

        let target_samples = self.chunk_frames * self.spec.channels;

        while self.pending_samples.len() < target_samples && !self.finished {
            let packet = match self.format.next_packet() {
                Ok(packet) => packet,

                Err(Error::ResetRequired) => {
                    return Err("Decoder reset required".into());
                }

                Err(Error::IoError(_)) => {
                    self.finished = true;
                    break;
                }

                Err(error) => {
                    return Err(Box::new(error));
                }
            };

            if packet.track_id() != self.track_id {
                continue;
            }

            let decoded = self.decoder.decode(&packet)?;

            let decoded_spec = *decoded.spec();

            let actual_spec = AudioSpec::new(
                decoded_spec.rate,
                decoded_spec.channels.count(),
            );

            if actual_spec != self.spec {
                return Err(
                    "Audio format changed during decoding".into()
                );
            }

            let duration = decoded.capacity() as u64;

            let mut buffer =
                SampleBuffer::<f32>::new(duration, decoded_spec);

            buffer.copy_interleaved_ref(decoded);

            self.pending_samples
                .extend(buffer.samples().iter().copied());
        }

        if self.pending_samples.is_empty() {
            return Ok(None);
        }

        let samples_to_take =
            self.pending_samples.len().min(target_samples);

        let samples = self
            .pending_samples
            .drain(..samples_to_take)
            .collect::<Vec<f32>>();

        Ok(Some(PcmBuffer::new(self.spec, samples)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_chunk_size() {
        let result = AudioDecoder::open_with_chunk_size(
            "nonexistent.flac",
            0,
        );

        assert!(result.is_err());
    }
}