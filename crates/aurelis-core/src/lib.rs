pub mod audio;

pub const NAME: &str = "AURELIS";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn initialize() {
    println!("{NAME} core initialized — v{VERSION}");
}