use std::sync::mpsc;

use crate::Spout;

#[derive(Debug, Clone)]
pub struct ChannelSpout<T> {
    sender: mpsc::Sender<T>,
}

impl<T> ChannelSpout<T> {
    /// Create a new channel spout from a sender.
    pub fn new(sender: mpsc::Sender<T>) -> Self {
        Self { sender }
    }

    /// Get a reference to the underlying sender.
    pub fn sender(&self) -> &mpsc::Sender<T> {
        &self.sender
    }

    /// Consume the spout and return the sender.
    pub fn into_sender(self) -> mpsc::Sender<T> {
        self.sender
    }
}

impl<T> Spout<T> for ChannelSpout<T> {
    #[inline]
    fn send(&mut self, item: T) {
        // Ignore send errors - receiver may have been dropped
        let _ = self.sender.send(item);
    }
}

/// Bounded channel spout with backpressure.
///
/// Wraps a [`SyncSender`](mpsc::SyncSender) — when the channel is full,
/// `send()` blocks until the receiver drains an item. This provides
/// backpressure: the producer stalls rather than dropping data.
///
/// Use this as the outermost spout in a ring chain when you need
/// lossless delivery with flow control.
///
/// # Example
///
/// ```
/// use std::sync::mpsc;
/// use spout::{Spout, SyncChannelSpout};
///
/// let (tx, rx) = mpsc::sync_channel(4);
/// let mut spout = SyncChannelSpout::new(tx);
///
/// spout.send(1);
/// spout.send(2);
/// assert_eq!(rx.recv().unwrap(), 1);
/// assert_eq!(rx.recv().unwrap(), 2);
/// ```
#[derive(Debug, Clone)]
pub struct SyncChannelSpout<T> {
    sender: mpsc::SyncSender<T>,
}

impl<T> SyncChannelSpout<T> {
    /// Create a new sync channel spout from a sender.
    pub fn new(sender: mpsc::SyncSender<T>) -> Self {
        Self { sender }
    }

    /// Get a reference to the underlying sender.
    pub fn sender(&self) -> &mpsc::SyncSender<T> {
        &self.sender
    }

    /// Consume the spout and return the sender.
    pub fn into_sender(self) -> mpsc::SyncSender<T> {
        self.sender
    }
}

impl<T> Spout<T> for SyncChannelSpout<T> {
    #[inline]
    fn send(&mut self, item: T) {
        // Blocks when channel is full — this IS the backpressure.
        // Ignores errors if receiver is dropped.
        let _ = self.sender.send(item);
    }
}

/// Thread-safe spout wrapper using `Arc<Mutex<S>>`.
///
/// Allows multiple producers to share a single spout with mutex synchronization.
/// Useful for MPSC patterns where all items should go to one collector.
impl<T, S: Spout<T>> Spout<T> for std::sync::Arc<std::sync::Mutex<S>> {
    #[inline]
    fn send(&mut self, item: T) {
        self.lock().unwrap().send(item);
    }

    #[inline]
    fn flush(&mut self) {
        self.lock().unwrap().flush();
    }
}
