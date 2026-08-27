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

    match aurelis_core::audio::decoder::decode_file(&path) {
        Ok(audio) => {
            let duration = audio.samples.len() as f64
                / audio.spec.channels as f64
                / audio.spec.sample_rate as f64;

            println!("Decoded successfully.");
            println!("Sample rate : {} Hz", audio.spec.sample_rate);
            println!("Channels    : {}", audio.spec.channels);
            println!("Samples     : {}", audio.samples.len());
            println!("Duration    : {:.2} seconds", duration);
        }

        Err(error) => {
            eprintln!("Failed to decode audio: {error}");
        }
    }
}