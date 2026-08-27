use std::env;

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

    let mut decoder =
        match aurelis_core::audio::AudioDecoder::open(&path) {
            Ok(decoder) => decoder,
            Err(error) => {
                eprintln!("Failed to open audio: {error}");
                return;
            }
        };

    let spec = decoder.spec();

    println!("Audio opened successfully.");
    println!("Sample rate : {} Hz", spec.sample_rate);
    println!("Channels    : {}", spec.channels);
    println!("Chunk size  : {} frames", decoder.chunk_frames());

    let mut chunk_count = 0usize;
    let mut total_frames = 0usize;

    while let Some(chunk) = match decoder.next_chunk() {
        Ok(chunk) => chunk,
        Err(error) => {
            eprintln!("Decoding error: {error}");
            return;
        }
    } {
        chunk_count += 1;
        total_frames += chunk.frame_count();

        if chunk_count <= 3 || chunk_count % 100 == 0 {
            println!(
                "Chunk {:>4}: {:>6} frames",
                chunk_count,
                chunk.frame_count()
            );
        }
    }

    let duration = total_frames as f64 / spec.sample_rate as f64;

    println!();
    println!("Streaming decode complete.");
    println!("Chunks      : {}", chunk_count);
    println!("Total frames: {}", total_frames);
    println!("Duration    : {:.2} seconds", duration);
    println!("Finished    : {}", decoder.is_finished());
}