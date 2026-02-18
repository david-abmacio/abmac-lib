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
    type Error = mpsc::SendError<T>;

    #[inline]
    fn send(&mut self, item: T) -> Result<(), Self::Error> {
        self.sender.send(item)
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
    type Error = mpsc::SendError<T>;

    #[inline]
    fn send(&mut self, item: T) -> Result<(), Self::Error> {
        // Blocks when channel is full — this IS the backpressure.
        self.sender.send(item)
    }
}

/// Error from an `Arc<Mutex<S>>` spout.
///
/// Wraps either the inner spout's error or a mutex poison error.
#[derive(Debug)]
pub enum MutexSpoutError<E> {
    /// The inner spout returned an error.
    Spout(E),
    /// The mutex was poisoned by a panicked thread.
    Poisoned,
}

impl<E: core::fmt::Display> core::fmt::Display for MutexSpoutError<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Spout(e) => write!(f, "{e}"),
            Self::Poisoned => write!(f, "mutex poisoned"),
        }
    }
}

impl<E: core::fmt::Debug + core::fmt::Display> core::error::Error for MutexSpoutError<E> {}

/// Thread-safe spout wrapper using `Arc<Mutex<S>>`.
///
/// Allows multiple producers to share a single spout with mutex synchronization.
/// Useful for MPSC patterns where all items should go to one collector.
impl<T, S: Spout<T>> Spout<T> for std::sync::Arc<std::sync::Mutex<S>> {
    type Error = MutexSpoutError<S::Error>;

    #[inline]
    fn send(&mut self, item: T) -> Result<(), Self::Error> {
        self.lock()
            .map_err(|_| MutexSpoutError::Poisoned)?
            .send(item)
            .map_err(MutexSpoutError::Spout)
    }

    #[inline]
    fn flush(&mut self) -> Result<(), Self::Error> {
        self.lock()
            .map_err(|_| MutexSpoutError::Poisoned)?
            .flush()
            .map_err(MutexSpoutError::Spout)
    }
}
