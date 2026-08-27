#[derive(Debug, Clone, Copy)]
pub struct AudioSpec {
    pub sample_rate: u32,
    pub channels: usize,
}

impl AudioSpec {
    pub const fn new(sample_rate: u32, channels: usize) -> Self {
        Self {
            sample_rate,
            channels,
        }
    }
}