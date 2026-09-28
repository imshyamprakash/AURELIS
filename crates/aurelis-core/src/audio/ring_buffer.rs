use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct AudioRingBuffer {
    buffer: Box<[f32]>,
    capacity: usize,
    read: AtomicUsize,
    write: AtomicUsize,
}

impl AudioRingBuffer {
    pub fn new(capacity: usize) -> Arc<Self> {
        assert!(capacity > 1);

        Arc::new(Self {
            buffer: vec![0.0; capacity].into_boxed_slice(),
            capacity,
            read: AtomicUsize::new(0),
            write: AtomicUsize::new(0),
        })
    }

    pub fn push(&self, samples: &[f32]) -> usize {
        let mut written = 0;

        while written < samples.len() {
            let read = self.read.load(Ordering::Acquire);
            let write = self.write.load(Ordering::Relaxed);

            let next = (write + 1) % self.capacity;

            if next == read {
                break;
            }

            unsafe {
                let ptr = self.buffer.as_ptr() as *mut f32;
                *ptr.add(write) = samples[written];
            }

            self.write.store(next, Ordering::Release);
            written += 1;
        }

        written
    }

    pub fn pop(&self, output: &mut [f32]) -> usize {
        let mut read_count = 0;

        while read_count < output.len() {
            let read = self.read.load(Ordering::Relaxed);
            let write = self.write.load(Ordering::Acquire);

            if read == write {
                break;
            }

            output[read_count] = self.buffer[read];

            self.read
                .store((read + 1) % self.capacity, Ordering::Release);

            read_count += 1;
        }

        read_count
    }

    pub fn available_read(&self) -> usize {
        let read = self.read.load(Ordering::Acquire);
        let write = self.write.load(Ordering::Acquire);

        if write >= read {
            write - read
        } else {
            self.capacity - read + write
        }
    }

    pub fn available_write(&self) -> usize {
        self.capacity - self.available_read() - 1
    }

    pub fn clear(&self) {
        self.read.store(0, Ordering::Release);
        self.write.store(0, Ordering::Release);
    }
}