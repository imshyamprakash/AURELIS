use std::fs::File;
use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::default::{get_codecs, get_probe};

use super::types::{AudioSpec, PcmBuffer};

pub fn decode_file<P: AsRef<Path>>(
    path: P,
) -> Result<PcmBuffer, Box<dyn std::error::Error>> {
    let file = File::open(path)?;

    let media_source =
        MediaSourceStream::new(Box::new(file), Default::default());

    let hint = Hint::new();

    let probed = get_probe().format(
        &hint,
        media_source,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;

    let mut format = probed.format;

    let track = format
        .default_track()
        .ok_or("No audio track found")?;

    let track_id = track.id;
    let codec_params = track.codec_params.clone();

    let mut decoder =
        get_codecs().make(&codec_params, &DecoderOptions::default())?;

    let initial_sample_rate = codec_params
        .sample_rate
        .ok_or("Missing sample rate")?;

    let initial_channels = codec_params
        .channels
        .ok_or("Missing channel information")?
        .count();

    let mut spec =
        AudioSpec::new(initial_sample_rate, initial_channels);

    let mut samples = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,

            Err(Error::ResetRequired) => {
                return Err("Decoder reset required".into());
            }

            Err(Error::IoError(_)) => {
                break;
            }

            Err(error) => {
                return Err(Box::new(error));
            }
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = decoder.decode(&packet)?;

        let decoded_spec = *decoded.spec();

        spec = AudioSpec::new(
            decoded_spec.rate,
            decoded_spec.channels.count(),
        );

        let duration = decoded.capacity() as u64;

        let mut buffer =
            SampleBuffer::<f32>::new(duration, decoded_spec);

        buffer.copy_interleaved_ref(decoded);

        samples.extend_from_slice(buffer.samples());
    }

    Ok(PcmBuffer::new(spec, samples))
}