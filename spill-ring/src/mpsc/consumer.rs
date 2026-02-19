extern crate alloc;

use crate::SpillRing;
use alloc::vec::Vec;
use spout::Spout;

use super::Producer;

/// Consumer handle for an MPSC ring.
///
/// Collects producer rings and provides drain functionality.
/// Use this when you need manual control over when items are consumed.
pub struct Consumer<
    T,
    const N: usize,
    S: Spout<T, Error = core::convert::Infallible> = spout::DropSpout,
> {
    rings: Vec<SpillRing<T, N, S>>,
}

impl<T, const N: usize, S: Spout<T, Error = core::convert::Infallible>> Consumer<T, N, S> {
    pub(crate) fn new() -> Self {
        Self { rings: Vec::new() }
    }

    pub(crate) fn add_ring(&mut self, ring: SpillRing<T, N, S>) {
        self.rings.push(ring);
    }

    /// Collect producers back into this consumer for draining.
    ///
    /// Reunites producers with their consumer after threads complete.
    pub fn collect(&mut self, producers: impl IntoIterator<Item = Producer<T, N, S>>) {
        for producer in producers {
            self.add_ring(producer.into_ring());
        }
    }

    /// Drain all items from all rings into a spout.
    ///
    /// Items are drained in producer order, then FIFO within each producer.
    pub fn drain<Out: Spout<T, Error = core::convert::Infallible>>(
        &mut self,
        spout: &mut Out,
    ) {
        for ring in &mut self.rings {
            let _ = spout.send_all(ring.drain());
        }
        let _ = spout.flush();
    }

    /// Get the number of producers/rings.
    #[must_use]
    pub fn num_producers(&self) -> usize {
        self.rings.len()
    }

    /// Check if all rings are empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rings.iter().all(SpillRing::is_empty)
    }

    /// Get total items across all rings.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rings.iter().map(SpillRing::len).sum()
    }
}
