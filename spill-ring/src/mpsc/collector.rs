//! Collector strategy for `FanInSpout` batch delivery.
//!
//! The `Collector` trait decouples how `FanInSpout` delivers collected
//! batches from the downstream spout interface. This bridges the gap
//! between batch-level metadata (`batch_seq`, `worker_id`) and the
//! item-level `Spout<T>` trait.

use spout::Spout;

/// Strategy for how `FanInSpout` delivers collected batches.
///
/// Each `flush()` cycle in `FanInSpout` calls `deliver()` for each slot
/// that has a published batch, then calls `finish()` once all slots
/// have been processed.
pub trait Collector<T> {
    /// Error type for delivery failures.
    type Error;

    /// Deliver a batch of items with metadata.
    ///
    /// - `batch_seq`: sequence number stamped by `dispatch()` (monotonic)
    /// - `worker_id`: index of the worker that produced this batch
    /// - `items`: iterator over the batch contents (drained from ring)
    fn deliver(
        &mut self,
        batch_seq: u64,
        worker_id: usize,
        items: impl Iterator<Item = T>,
    ) -> Result<(), Self::Error>;

    /// Signal that all available batches have been delivered for this flush.
    ///
    /// Called once per `flush()` cycle after all `deliver()` calls complete.
    /// Use this to flush downstream buffers or emit end-of-batch markers.
    fn finish(&mut self) -> Result<(), Self::Error>;
}

/// Unordered collector: forwards items directly to an inner spout.
///
/// Ignores `batch_seq` and `worker_id` — same behavior as
/// `WorkerPool::collect()`. This is the default collector for
/// workloads that don't need ordering.
pub struct UnorderedCollector<S> {
    inner: S,
}

impl<S> UnorderedCollector<S> {
    /// Wrap a spout as an unordered collector.
    pub fn new(inner: S) -> Self {
        Self { inner }
    }

    /// Reference to the inner spout.
    pub fn inner(&self) -> &S {
        &self.inner
    }

    /// Mutable reference to the inner spout.
    pub fn inner_mut(&mut self) -> &mut S {
        &mut self.inner
    }

    /// Consume and return the inner spout.
    pub fn into_inner(self) -> S {
        self.inner
    }
}

impl<T, S: Spout<T>> Collector<T> for UnorderedCollector<S> {
    type Error = S::Error;

    #[inline]
    fn deliver(
        &mut self,
        _batch_seq: u64,
        _worker_id: usize,
        items: impl Iterator<Item = T>,
    ) -> Result<(), Self::Error> {
        self.inner.send_all(items)
    }

    #[inline]
    fn finish(&mut self) -> Result<(), Self::Error> {
        self.inner.flush()
    }
}

/// Sequenced collector: reorders batches via `SequencedSpout` before forwarding.
///
/// Routes each batch through `SequencedSpout::submit(batch_seq, items)`,
/// which buffers out-of-order batches and emits in strict sequence order.
pub struct SequencedCollector<T, S: Spout<T>> {
    sequencer: spout::SequencedSpout<T, S>,
}

impl<T, S: Spout<T>> SequencedCollector<T, S> {
    /// Wrap a `SequencedSpout` as a collector.
    pub fn new(sequencer: spout::SequencedSpout<T, S>) -> Self {
        Self { sequencer }
    }

    /// Create from an inner spout (starting at sequence 0).
    pub fn from_spout(inner: S) -> Self {
        Self {
            sequencer: spout::SequencedSpout::new(inner),
        }
    }

    /// Reference to the inner `SequencedSpout`.
    pub fn sequencer(&self) -> &spout::SequencedSpout<T, S> {
        &self.sequencer
    }

    /// Mutable reference to the inner `SequencedSpout`.
    pub fn sequencer_mut(&mut self) -> &mut spout::SequencedSpout<T, S> {
        &mut self.sequencer
    }

    /// Consume and return the inner `SequencedSpout`.
    pub fn into_sequencer(self) -> spout::SequencedSpout<T, S> {
        self.sequencer
    }
}

impl<T, S: Spout<T>> Collector<T> for SequencedCollector<T, S> {
    type Error = S::Error;

    #[inline]
    fn deliver(
        &mut self,
        batch_seq: u64,
        _worker_id: usize,
        items: impl Iterator<Item = T>,
    ) -> Result<(), Self::Error> {
        self.sequencer.submit(batch_seq, items)
    }

    #[inline]
    fn finish(&mut self) -> Result<(), Self::Error> {
        // All workers for this dispatch round have been delivered.
        // Advance the sequencer past the current sequence, flushing
        // any buffered batches that are now in order.
        self.sequencer.advance()?;
        self.sequencer.inner_mut().flush()
    }
}
