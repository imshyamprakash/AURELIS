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

#[derive(Debug)]
pub struct PcmBuffer {
    pub spec: AudioSpec,
    pub samples: Vec<f32>,
}

impl PcmBuffer {
    pub fn new(spec: AudioSpec, samples: Vec<f32>) -> Self {
        Self { spec, samples }
    }

    pub fn frame_count(&self) -> usize {
        if self.spec.channels == 0 {
            return 0;
        }

        self.samples.len() / self.spec.channels
    }

    pub fn duration_seconds(&self) -> f64 {
        if self.spec.sample_rate == 0 {
            return 0.0;
        }

        self.frame_count() as f64 / self.spec.sample_rate as f64
    }
}