//! Reordering spout for ordered completion.
//!
//! `SequencedSpout` buffers out-of-order batches and emits them in strict
//! sequence order. This is the io_uring completion queue pattern —
//! parallel submission, ordered completion — generalized through the
//! spout trait.

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use crate::Spout;

/// Reorders batches by sequence number before forwarding to an inner spout.
///
/// Fast path: if the next expected sequence arrives, emit directly (zero
/// buffering). Slow path: buffer until predecessors arrive, then flush
/// in order.
///
/// # Buffering Bound
///
/// The reordering buffer size is bounded by the maximum out-of-order
/// distance between producers. With N workers, at most N-1 batches can
/// be buffered at any time (the fastest producer is at most N-1 batches
/// ahead of the slowest).
pub struct SequencedSpout<T, Inner: Spout<T>> {
    next_expected: u64,
    pending: BTreeMap<u64, Vec<T>>,
    inner: Inner,
}

impl<T, Inner: Spout<T>> SequencedSpout<T, Inner> {
    /// Create a new sequenced spout wrapping an inner spout.
    ///
    /// Batches are reordered by sequence number before forwarding.
    /// The first expected sequence number is 0.
    pub fn new(inner: Inner) -> Self {
        Self {
            next_expected: 0,
            pending: BTreeMap::new(),
            inner,
        }
    }

    /// Create with a custom starting sequence number.
    pub fn with_start_seq(inner: Inner, start_seq: u64) -> Self {
        Self {
            next_expected: start_seq,
            pending: BTreeMap::new(),
            inner,
        }
    }

    /// Submit a batch with its sequence number.
    ///
    /// If `batch_seq >= next_expected` and no gap exists, items are emitted
    /// directly to the inner spout (fast path, zero buffering). Multiple
    /// submits with the same sequence number all emit directly — this
    /// supports N workers sharing the same dispatch-round sequence.
    ///
    /// If `batch_seq > next_expected` (gap), items are buffered in a
    /// `BTreeMap` until their predecessors arrive. Multiple submits for
    /// the same future sequence are appended, not overwritten.
    ///
    /// Call [`advance()`](Self::advance) after all workers for a given
    /// sequence have submitted to move `next_expected` forward.
    pub fn submit(
        &mut self,
        batch_seq: u64,
        items: impl Iterator<Item = T>,
    ) -> Result<(), Inner::Error> {
        if batch_seq == self.next_expected {
            // Fast path: in order, emit directly.
            for item in items {
                self.inner.send(item)?;
            }
            // Do NOT advance next_expected here — multiple workers may
            // share the same seq. The caller advances via advance().
        } else if batch_seq > self.next_expected {
            // Out of order: buffer until predecessors arrive.
            // Append to existing entry (multiple workers per seq).
            self.pending.entry(batch_seq).or_default().extend(items);
        } else {
            // batch_seq < next_expected: already advanced past this seq.
            // This shouldn't happen in normal usage, but emit directly
            // to avoid silent data loss.
            for item in items {
                self.inner.send(item)?;
            }
        }
        Ok(())
    }

    /// Advance past the current sequence, flushing any buffered batches
    /// that are now in order.
    ///
    /// Call this after all workers for the current sequence have submitted.
    /// For the TPC pool, call once per `flush()` cycle after iterating all
    /// handoff slots.
    pub fn advance(&mut self) -> Result<(), Inner::Error> {
        self.next_expected += 1;
        // Drain any buffered batches that are now in order.
        while let Some(buffered) = self.pending.remove(&self.next_expected) {
            for item in buffered {
                self.inner.send(item)?;
            }
            self.next_expected += 1;
        }
        Ok(())
    }

    /// Number of batches currently buffered (waiting for predecessors).
    pub fn buffered_batches(&self) -> usize {
        self.pending.len()
    }

    /// The next expected sequence number.
    pub fn next_expected(&self) -> u64 {
        self.next_expected
    }

    /// Reference to the inner spout.
    pub fn inner(&self) -> &Inner {
        &self.inner
    }

    /// Mutable reference to the inner spout.
    pub fn inner_mut(&mut self) -> &mut Inner {
        &mut self.inner
    }

    /// Consume and return the inner spout.
    ///
    /// Any buffered batches are dropped. Call `submit()` for all
    /// remaining sequences first to ensure nothing is lost.
    pub fn into_inner(self) -> Inner {
        self.inner
    }
}
