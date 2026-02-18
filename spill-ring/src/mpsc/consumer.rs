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

    /// Drain all items from all rings into a sink.
    ///
    /// Items are drained in producer order, then FIFO within each producer.
    pub fn drain<Spout2: Spout<T, Error = core::convert::Infallible>>(
        &mut self,
        sink: &mut Spout2,
    ) {
        for ring in &mut self.rings {
            let _ = sink.send_all(ring.drain());
        }
        let _ = sink.flush();
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

/// Collect producers back into a consumer for draining.
///
/// This is a helper to reunite producers with their consumer after threads complete.
pub fn collect<T, const N: usize, S: Spout<T, Error = core::convert::Infallible>>(
    producers: impl IntoIterator<Item = Producer<T, N, S>>,
    consumer: &mut Consumer<T, N, S>,
) {
    for producer in producers {
        consumer.add_ring(producer.into_ring());
    }
}
