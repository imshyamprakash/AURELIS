use std::sync::atomic::{AtomicUsize, Ordering};

/// Lock-free single-producer/single-consumer ring buffer.
///
/// This is designed for AURELIS's audio thread:
///
/// Decoder Thread  -> push()
/// Audio Callback  -> pop()
///
/// No mutexes are used.
pub struct AudioRingBuffer {
    buffer: Box<[f32]>,
    capacity: usize,
    write: AtomicUsize,
    read: AtomicUsize,
}

impl AudioRingBuffer {
    /// Create a new ring buffer.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 1, "capacity must be greater than 1");

        Self {
            buffer: vec![0.0; capacity].into_boxed_slice(),
            capacity,
            write: AtomicUsize::new(0),
            read: AtomicUsize::new(0),
        }
    }

    /// Maximum number of samples.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Number of samples currently stored.
    pub fn len(&self) -> usize {
        let w = self.write.load(Ordering::Acquire);
        let r = self.read.load(Ordering::Acquire);

        if w >= r {
            w - r
        } else {
            self.capacity - r + w
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn is_full(&self) -> bool {
        self.len() == self.capacity - 1
    }

    /// Push samples into the buffer.
    ///
    /// Returns the number of samples successfully written.
    pub fn push(&self, input: &[f32]) -> usize {
        let mut written = 0;

        while written < input.len() {
            let w = self.write.load(Ordering::Relaxed);
            let r = self.read.load(Ordering::Acquire);

            let next = (w + 1) % self.capacity;

            if next == r {
                break;
            }

            unsafe {
                let ptr = self.buffer.as_ptr() as *mut f32;
                ptr.add(w).write(input[written]);
            }

            self.write.store(next, Ordering::Release);
            written += 1;
        }

        written
    }

    /// Pop samples from the buffer.
    ///
    /// Returns the number of samples read.
    pub fn pop(&self, output: &mut [f32]) -> usize {
        let mut read_count = 0;

        while read_count < output.len() {
            let r = self.read.load(Ordering::Relaxed);
            let w = self.write.load(Ordering::Acquire);

            if r == w {
                break;
            }

            output[read_count] = unsafe {
                *self.buffer.as_ptr().add(r)
            };

            self.read
                .store((r + 1) % self.capacity, Ordering::Release);

            read_count += 1;
        }

        read_count
    }

    /// Remove everything.
    pub fn clear(&self) {
        let w = self.write.load(Ordering::Acquire);
        self.read.store(w, Ordering::Release);
    }

    /// Remaining free space.
    pub fn available_space(&self) -> usize {
        self.capacity - 1 - self.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_buffer_is_empty() {
        let rb = AudioRingBuffer::new(8);

        assert!(rb.is_empty());
        assert_eq!(rb.len(), 0);
        assert_eq!(rb.available_space(), 7);
    }

    #[test]
    fn push_and_pop_preserves_order() {
        let rb = AudioRingBuffer::new(8);

        let input = [1.0, 2.0, 3.0, 4.0];

        assert_eq!(rb.push(&input), 4);

        let mut out = [0.0; 4];

        assert_eq!(rb.pop(&mut out), 4);

        assert_eq!(input, out);
    }

    #[test]
    fn wraparound_works() {
        let rb = AudioRingBuffer::new(8);

        let first = [1.0, 2.0, 3.0, 4.0];
        rb.push(&first);

        let mut temp = [0.0; 2];
        rb.pop(&mut temp);

        let second = [5.0, 6.0, 7.0];
        rb.push(&second);

        let mut out = [0.0; 5];
        rb.pop(&mut out);

        assert_eq!(out, [3.0, 4.0, 5.0, 6.0, 7.0]);
    }

    #[test]
    fn clear_empties_buffer() {
        let rb = AudioRingBuffer::new(8);

        rb.push(&[1.0, 2.0, 3.0]);

        rb.clear();

        assert!(rb.is_empty());
        assert_eq!(rb.len(), 0);
    }

    #[test]
    fn full_buffer_rejects_extra_samples() {
        let rb = AudioRingBuffer::new(4);

        assert_eq!(rb.push(&[1.0, 2.0, 3.0]), 3);
        assert!(rb.is_full());

        assert_eq!(rb.push(&[4.0]), 0);
    }
}