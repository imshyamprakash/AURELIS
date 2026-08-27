use std::collections::VecDeque;

use super::types::{AudioSpec, PcmBuffer};

/// Number of PCM frames produced by one decoder iteration.
///
/// 4096 frames at 96 kHz is approximately 42.7 ms of audio.
/// This gives us a reasonable starting point for low-latency processing.
pub const DEFAULT_CHUNK_FRAMES: usize = 4096;

/// A bounded queue of PCM audio waiting to be consumed.
///
/// The queue owns only a limited amount of decoded audio.
/// This prevents the audio pipeline from growing without bounds.
#[derive(Debug)]
pub struct PcmStream {
    spec: AudioSpec,
    chunks: VecDeque<PcmBuffer>,
    queued_frames: usize,
    max_frames: usize,
}

impl PcmStream {
    /// Create a new PCM stream with a maximum number of queued frames.
    pub fn new(spec: AudioSpec, max_frames: usize) -> Self {
        Self {
            spec,
            chunks: VecDeque::new(),
            queued_frames: 0,
            max_frames,
        }
    }

    /// Return the audio specification used by this stream.
    pub const fn spec(&self) -> AudioSpec {
        self.spec
    }

    /// Return the number of PCM frames currently queued.
    pub const fn queued_frames(&self) -> usize {
        self.queued_frames
    }

    /// Return the maximum number of frames this stream can hold.
    pub const fn max_frames(&self) -> usize {
        self.max_frames
    }

    /// Return true when the stream has reached its capacity.
    pub const fn is_full(&self) -> bool {
        self.queued_frames >= self.max_frames
    }

    /// Return true when the stream contains no PCM frames.
    pub const fn is_empty(&self) -> bool {
        self.queued_frames == 0
    }

    /// Return the remaining frame capacity.
    pub const fn remaining_frames(&self) -> usize {
        self.max_frames.saturating_sub(self.queued_frames)
    }

    /// Push a PCM buffer into the stream.
    ///
    /// The buffer is rejected when:
    /// - Its audio specification does not match the stream.
    /// - Adding it would exceed the stream's maximum capacity.
    ///
    /// Returns `true` when accepted.
    pub fn push(&mut self, buffer: PcmBuffer) -> bool {
        if buffer.spec.sample_rate != self.spec.sample_rate
            || buffer.spec.channels != self.spec.channels
        {
            return false;
        }

        let frames = buffer.frame_count();

        if frames == 0 {
            return true;
        }

        if self.queued_frames + frames > self.max_frames {
            return false;
        }

        self.queued_frames += frames;
        self.chunks.push_back(buffer);

        true
    }

    /// Pop the oldest PCM buffer from the stream.
    pub fn pop(&mut self) -> Option<PcmBuffer> {
        let buffer = self.chunks.pop_front()?;

        self.queued_frames = self
            .queued_frames
            .saturating_sub(buffer.frame_count());

        Some(buffer)
    }

    /// Remove all queued PCM data.
    pub fn clear(&mut self) {
        self.chunks.clear();
        self.queued_frames = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::types::AudioSpec;

    fn test_buffer(frame_count: usize) -> PcmBuffer {
        let spec = AudioSpec::new(48_000, 2);
        let samples = vec![0.0; frame_count * spec.channels];

        PcmBuffer::new(spec, samples)
    }

    #[test]
    fn stream_accepts_buffer_within_capacity() {
        let spec = AudioSpec::new(48_000, 2);
        let mut stream = PcmStream::new(spec, 1_000);

        assert!(stream.push(test_buffer(500)));
        assert_eq!(stream.queued_frames(), 500);
        assert_eq!(stream.remaining_frames(), 500);
        assert!(!stream.is_empty());
    }

    #[test]
    fn stream_rejects_buffer_over_capacity() {
        let spec = AudioSpec::new(48_000, 2);
        let mut stream = PcmStream::new(spec, 1_000);

        assert!(!stream.push(test_buffer(1_001)));
        assert!(stream.is_empty());
        assert_eq!(stream.queued_frames(), 0);
        assert_eq!(stream.remaining_frames(), 1_000);
    }

    #[test]
    fn pop_removes_queued_frames() {
        let spec = AudioSpec::new(48_000, 2);
        let mut stream = PcmStream::new(spec, 1_000);

        assert!(stream.push(test_buffer(400)));
        assert_eq!(stream.queued_frames(), 400);

        let buffer = stream.pop();

        assert!(buffer.is_some());
        assert_eq!(stream.queued_frames(), 0);
        assert_eq!(stream.remaining_frames(), 1_000);
        assert!(stream.is_empty());
    }

    #[test]
    fn stream_rejects_mismatched_audio_spec() {
        let stream_spec = AudioSpec::new(48_000, 2);
        let buffer_spec = AudioSpec::new(96_000, 2);

        let mut stream = PcmStream::new(stream_spec, 1_000);

        let buffer = PcmBuffer::new(
            buffer_spec,
            vec![0.0; 200],
        );

        assert!(!stream.push(buffer));
        assert!(stream.is_empty());
    }

    #[test]
    fn stream_reports_full_when_capacity_is_reached() {
        let spec = AudioSpec::new(48_000, 2);
        let mut stream = PcmStream::new(spec, 1_000);

        assert!(stream.push(test_buffer(1_000)));
        assert!(stream.is_full());
        assert_eq!(stream.queued_frames(), 1_000);
        assert_eq!(stream.remaining_frames(), 0);
    }

    #[test]
    fn clear_removes_all_audio() {
        let spec = AudioSpec::new(48_000, 2);
        let mut stream = PcmStream::new(spec, 1_000);

        assert!(stream.push(test_buffer(500)));
        assert!(!stream.is_empty());

        stream.clear();

        assert!(stream.is_empty());
        assert_eq!(stream.queued_frames(), 0);
        assert_eq!(stream.remaining_frames(), 1_000);
    }
}