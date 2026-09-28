use std::env;

use aurelis_core::audio::PlaybackEngine;

fn main() {
    aurelis_core::initialize();

    let path = match env::args().nth(1) {
        Some(path) => path,
        None => {
            eprintln!("Usage: aurelis-app <path-to-audio-file>");
            return;
        }
    };

    println!("Opening: {}", path);

    let mut engine = match PlaybackEngine::open(&path) {
        Ok(engine) => engine,
        Err(error) => {
            eprintln!("Failed to initialize playback: {}", error);
            return;
        }
    };

    let source = engine.source_spec();
    let output = engine.output_spec();

    println!(
        "Source : {} Hz | {} ch",
        source.sample_rate,
        source.channels
    );
    println!(
        "Output : {} Hz | {} ch",
        output.sample_rate,
        output.channels
    );

    if engine.is_resampling() {
        println!(
            "Resampling: {} → {} Hz",
            source.sample_rate,
            output.sample_rate
        );
    } else {
        println!("Resampling: not required");
    }

    println!("Starting playback...");

    if let Err(error) = engine.play(path.clone()) {
        eprintln!("Playback failed: {}", error);
        return;
    }

    // Wait for the dedicated playback thread to finish.
    engine.wait();

    println!("Playback complete.");
}