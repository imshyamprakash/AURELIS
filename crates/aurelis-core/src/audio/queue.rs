use std::collections::VecDeque;
use std::path::{Path, PathBuf};

/// Playback queue.
///
/// This manages the ordered list of upcoming tracks.
/// Future versions will add:
/// - Shuffle
/// - Repeat
/// - Crossfade preparation
/// - Smart Queue
#[derive(Debug, Default)]
pub struct AudioQueue {
    tracks: VecDeque<PathBuf>,
}

impl AudioQueue {
    /// Create an empty queue.
    pub fn new() -> Self {
        Self {
            tracks: VecDeque::new(),
        }
    }

    /// Add a track to the end of the queue.
    pub fn push<P: AsRef<Path>>(&mut self, path: P) {
        self.tracks.push_back(path.as_ref().to_path_buf());
    }

    /// Remove and return the next track.
    pub fn pop(&mut self) -> Option<PathBuf> {
        self.tracks.pop_front()
    }

    /// View the next track without removing it.
    pub fn peek(&self) -> Option<&Path> {
        self.tracks.front().map(PathBuf::as_path)
    }

    /// Remove all tracks.
    pub fn clear(&mut self) {
        self.tracks.clear();
    }

    /// Number of queued tracks.
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    /// Check whether the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    /// Return all queued tracks.
    pub fn tracks(&self) -> impl Iterator<Item = &PathBuf> {
        self.tracks.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_queue_is_empty() {
        let queue = AudioQueue::new();

        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn push_increases_length() {
        let mut queue = AudioQueue::new();

        queue.push("song1.flac");
        queue.push("song2.mp3");

        assert_eq!(queue.len(), 2);
        assert!(!queue.is_empty());
    }

    #[test]
    fn peek_does_not_remove_track() {
        let mut queue = AudioQueue::new();

        queue.push("song1.flac");

        assert_eq!(
            queue.peek().unwrap(),
            Path::new("song1.flac")
        );

        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn pop_returns_fifo_order() {
        let mut queue = AudioQueue::new();

        queue.push("song1.flac");
        queue.push("song2.mp3");
        queue.push("song3.ogg");

        assert_eq!(
            queue.pop().unwrap(),
            PathBuf::from("song1.flac")
        );
        assert_eq!(
            queue.pop().unwrap(),
            PathBuf::from("song2.mp3")
        );
        assert_eq!(
            queue.pop().unwrap(),
            PathBuf::from("song3.ogg")
        );

        assert!(queue.pop().is_none());
    }

    #[test]
    fn clear_removes_everything() {
        let mut queue = AudioQueue::new();

        queue.push("song1.flac");
        queue.push("song2.mp3");

        queue.clear();

        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);
    }
}