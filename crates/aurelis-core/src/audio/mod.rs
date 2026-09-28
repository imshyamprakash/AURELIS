//! AURELIS audio engine.
//!
//! Complete playback pipeline:
//! Decoder → DSP → Resampler → Playback → Output.

pub mod decoder;
pub mod decoder_stream;
pub mod output;
pub mod playback;
pub mod queue;
pub mod resampler;
pub mod ring_buffer;
pub mod stream;
pub mod types;

// Public API
pub use decoder_stream::AudioDecoder;
pub use output::AudioOutput;
pub use playback::{PlaybackEngine, PlaybackState};
pub use queue::AudioQueue;
pub use resampler::AudioResampler;
pub use ring_buffer::AudioRingBuffer;

// Export these directly from types.rs
pub use types::{AudioSpec, PcmBuffer};