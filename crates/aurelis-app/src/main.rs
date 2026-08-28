use std::env;
use std::thread;
use std::time::Duration;

use aurelis_core::audio::{AudioDecoder, AudioOutput, AudioResampler};

fn main() {
    aurelis_core::initialize();

    let path = match env::args().nth(1) {
        Some(path) => path,
        None => {
            eprintln!("Usage: aurelis-app <path-to-audio-file>");
            return;
        }
    };

    println!("Opening: {path}");

    let mut decoder = match AudioDecoder::open(&path) {
        Ok(decoder) => decoder,
        Err(error) => {
            eprintln!("Failed to open audio: {error}");
            return;
        }
    };

    let source_spec = decoder.spec();

    println!("Audio opened successfully.");
    println!("Source sample rate : {} Hz", source_spec.sample_rate);
    println!("Source channels    : {}", source_spec.channels);
    println!("Chunk size         : {} frames", decoder.chunk_frames());

    // Ask the device what it actually supports *before* opening it. The
    // source format must never dictate what the hardware has to provide;
    // instead we resample to whatever the device natively wants.
    let device_spec = match AudioOutput::default_device_spec() {
        Ok(spec) => spec,
        Err(error) => {
            eprintln!("Failed to query audio device: {error}");
            return;
        }
    };

    println!();
    println!("Output sample rate : {} Hz", device_spec.sample_rate);
    println!("Output channels    : {}", device_spec.channels);

    // Only build a resampler when the rates actually differ. Resampling
    // identical rates is unnecessary work, and AudioResampler::new()
    // rejects it outright.
    let mut resampler = if source_spec.sample_rate != device_spec.sample_rate
    {
        println!();
        println!(
            "Resampling enabled:\n    {} Hz -> {} Hz",
            source_spec.sample_rate, device_spec.sample_rate
        );

        match AudioResampler::new(source_spec, device_spec) {
            Ok(resampler) => Some(resampler),
            Err(error) => {
                eprintln!("Failed to create resampler: {error}");
                return;
            }
        }
    } else {
        println!();
        println!("Sample rates match — no resampling needed.");
        None
    };

    // AudioOutput always opens at the device's native spec. If we're
    // resampling, that's the resampler's output; if not, it's the same as
    // the source.
    let output = match AudioOutput::open(device_spec) {
        Ok(output) => output,
        Err(error) => {
            eprintln!("Failed to open audio output: {error}");
            return;
        }
    };

    println!("Audio output opened successfully.");
    println!("Starting playback...");
    println!();

    let mut chunk_count = 0usize;
    let mut total_source_frames = 0usize;
    let mut total_output_frames = 0usize;

    loop {
        let chunk = match decoder.next_chunk() {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(error) => {
                eprintln!("Decoding error: {error}");
                return;
            }
        };

        chunk_count += 1;
        total_source_frames += chunk.frame_count();

        let is_final_chunk = chunk.frame_count() < decoder.chunk_frames();

        let to_play = if let Some(resampler) = resampler.as_mut() {
            let result = if is_final_chunk {
                resampler.process_final(&chunk)
            } else {
                resampler.process(&chunk)
            };

            match result {
                Ok(resampled) => resampled,
                Err(error) => {
                    eprintln!("Resampling error: {error}");
                    return;
                }
            }
        } else {
            chunk
        };

        total_output_frames += to_play.frame_count();
        output.push_samples(&to_play.samples);

        if chunk_count <= 3 || chunk_count % 100 == 0 {
            println!(
                "Chunk {:>4}: {:>6} source frames -> {:>6} output frames | queued: {:>8} samples",
                chunk_count,
                to_play.frame_count().max(0),
                to_play.frame_count(),
                output.pending_samples()
            );
        }
    }

    // Drain any real audio still held in the resampler's filter delay
    // line. Must run exactly once, after the true final chunk, before the
    // resampler is dropped.
    if let Some(resampler) = resampler.as_mut() {
        match resampler.flush() {
            Ok(tail) => {
                total_output_frames += tail.frame_count();
                output.push_samples(&tail.samples);
            }
            Err(error) => {
                eprintln!("Failed to flush resampler: {error}");
                return;
            }
        }
    }

    println!();
    println!("Streaming decode complete.");
    println!("Chunks              : {}", chunk_count);
    println!("Total source frames : {}", total_source_frames);
    println!("Total output frames : {}", total_output_frames);

    let duration = total_source_frames as f64 / source_spec.sample_rate as f64;

    println!("Duration            : {:.2} seconds", duration);
    println!("Finished            : {}", decoder.is_finished());

    while output.pending_samples() > 0 {
        thread::sleep(Duration::from_millis(50));
    }

    println!("Playback complete.");
}