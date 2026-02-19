use crate::SpillRing;
use spout::{DropSpout, Spout};

/// A producer handle for an MPSC ring.
///
/// Each producer owns its own [`SpillRing`] with zero contention.
/// When dropped, remaining items stay in the ring for the consumer to drain.
pub struct Producer<T, const N: usize, S: Spout<T, Error = core::convert::Infallible> = DropSpout> {
    ring: SpillRing<T, N, S>,
}

impl<T, const N: usize, S: Spout<T, Error = core::convert::Infallible>> Producer<T, N, S> {
    /// Push an item to this producer's ring.
    ///
    /// This is the hot path - runs at ~4.6 Gelem/s.
    #[inline]
    pub fn push(&self, item: T) {
        self.ring.push(item);
    }

    /// Check if the ring is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }

    /// Check if the ring is full.
    #[inline]
    pub fn is_full(&self) -> bool {
        self.ring.is_full()
    }

    /// Get the number of items in the ring.
    #[inline]
    pub fn len(&self) -> usize {
        self.ring.len()
    }

    /// Get the ring's capacity.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.ring.capacity()
    }

    pub(crate) fn into_ring(self) -> SpillRing<T, N, S> {
        self.ring
    }
}

impl<T, const N: usize> Producer<T, N, DropSpout> {
    pub(crate) fn new() -> Self {
        Self {
            ring: SpillRing::new(),
        }
    }
}

impl<T, const N: usize, S: Spout<T, Error = core::convert::Infallible>> Producer<T, N, S> {
    pub(crate) fn with_spout(spout: S) -> Self {
        Self {
            ring: SpillRing::with_spout(spout),
        }
    }
}
