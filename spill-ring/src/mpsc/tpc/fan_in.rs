//! Composable fan-in spout for collecting from TPC handoff slots.
//!
//! `FanInSpout` iterates N handoff slots on `flush()`, drains any
//! published rings into a [`Collector`], and recycles the empty rings
//! back to workers. Created via [`WorkerPool::with_fan_in()`] (scoped,
//! safe) or [`WorkerPool::fan_in_unchecked()`] (unsafe, unscoped).

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;
use spout::Spout;

use super::collector::{Collector, UnorderedCollector};
use super::handoff::HandoffSlot;
use super::pool::SendPtr;

/// Collects from N handoff slots, delivers to a [`Collector`].
///
/// On [`flush()`](FanInSpout::flush), iterates all assigned handoff slots,
/// takes any published batch (Acquire), delivers items with metadata
/// (`batch_seq`, `worker_id`) to the collector, then recycles the empty
/// ring back to the worker (Release).
///
/// Created via [`WorkerPool::with_fan_in()`] (scoped, safe) or
/// [`WorkerPool::fan_in_unchecked()`] (unsafe, unscoped).
pub struct FanInSpout<
    T,
    const N: usize,
    S: Spout<T, Error = core::convert::Infallible>,
    C: Collector<T>,
> {
    /// Raw pointers to handoff slots owned by WorkerPool.
    /// Validity invariant: these point into the pool's `Box<[HandoffSlot]>`.
    /// The scoped API ensures the pool outlives this struct.
    slots: Box<[SendPtr<HandoffSlot<T, N, S>>]>,
    /// Collector strategy for batch delivery.
    collector: C,
}

impl<T, const N: usize, S, C> FanInSpout<T, N, S, C>
where
    S: Spout<T, Error = core::convert::Infallible>,
    C: Collector<T>,
{
    /// Create a FanInSpout from raw slot pointers and a collector.
    ///
    /// # Safety
    ///
    /// All pointers in `slots` must remain valid for the lifetime of
    /// this struct. The scoped API ([`WorkerPool::with_fan_in()`])
    /// guarantees this; the unsafe constructor
    /// ([`WorkerPool::fan_in_unchecked()`]) relies on the caller.
    pub(crate) unsafe fn new(slots: Box<[SendPtr<HandoffSlot<T, N, S>>]>, collector: C) -> Self {
        Self { slots, collector }
    }

    /// Reference to the collector.
    #[inline]
    pub fn collector(&self) -> &C {
        &self.collector
    }

    /// Mutable reference to the collector.
    #[inline]
    pub fn collector_mut(&mut self) -> &mut C {
        &mut self.collector
    }

    /// Consume and return the collector.
    #[inline]
    pub fn into_collector(self) -> C {
        self.collector
    }

    /// Number of handoff slots this fan-in collects from.
    #[inline]
    #[must_use]
    pub fn num_slots(&self) -> usize {
        self.slots.len()
    }

    /// Collect all available batches from handoff slots into the collector.
    ///
    /// For each slot with a published batch:
    /// 1. Swap null into batch pointer (Acquire) — takes ownership of ring
    /// 2. Read `batch_seq` from the slot
    /// 3. Deliver items with `(batch_seq, worker_id)` to the collector
    /// 4. Offer empty ring back to worker via recycle pointer (Release)
    ///
    /// Slots where the worker hasn't published yet are skipped silently.
    /// After all slots are processed, calls `collector.finish()`.
    pub fn flush(&mut self) -> Result<(), C::Error> {
        for (worker_id, slot_ptr) in self.slots.iter().enumerate() {
            // SAFETY: Pointer validity guaranteed by scoped API (with_fan_in)
            // or caller contract (fan_in_unchecked).
            let slot = unsafe { &*slot_ptr.0 };
            if let Some(mut ring) = slot.collect() {
                let batch_seq = slot.read_seq();
                self.collector.deliver(batch_seq, worker_id, ring.drain())?;
                slot.offer_recycle(ring);
            }
        }
        self.collector.finish()
    }

    pub(crate) fn slot_ptrs_from(
        slots: &[HandoffSlot<T, N, S>],
    ) -> Box<[SendPtr<HandoffSlot<T, N, S>>]> {
        slots
            .iter()
            .map(|slot| SendPtr(core::ptr::from_ref(slot)))
            .collect::<Vec<_>>()
            .into_boxed_slice()
    }
}

// --- Spout<T> impl for UnorderedCollector only (design decision D-6) ---

impl<T, const N: usize, S, Out> FanInSpout<T, N, S, UnorderedCollector<Out>>
where
    S: Spout<T, Error = core::convert::Infallible>,
    Out: Spout<T>,
{
    /// Reference to the inner spout (unordered collector only).
    #[inline]
    pub fn inner(&self) -> &Out {
        self.collector.inner()
    }

    /// Mutable reference to the inner spout (unordered collector only).
    #[inline]
    pub fn inner_mut(&mut self) -> &mut Out {
        self.collector.inner_mut()
    }

    /// Consume and return the inner spout (unordered collector only).
    #[inline]
    pub fn into_inner(self) -> Out {
        self.collector.into_inner()
    }
}

impl<T, const N: usize, S, Out> Spout<T> for FanInSpout<T, N, S, UnorderedCollector<Out>>
where
    S: Spout<T, Error = core::convert::Infallible>,
    Out: Spout<T>,
{
    type Error = Out::Error;

    /// Pass-through: forward an externally-produced item to the inner spout.
    ///
    /// This is NOT the primary data path — [`flush()`](FanInSpout::flush) is.
    #[inline]
    fn send(&mut self, item: T) -> Result<(), Self::Error> {
        self.collector.inner_mut().send(item)
    }

    /// Collect all available batches from handoff slots, drain into inner.
    fn flush(&mut self) -> Result<(), Self::Error> {
        // Delegate to the inherent flush() method which uses the Collector trait.
        FanInSpout::flush(self)
    }
}

// SAFETY: FanInSpout is Send when T and C are Send. flush() drains T items
// from handoff slots and delivers them to the collector, moving T values
// across thread boundaries. The slot pointers are SendPtr (manually Send).
// The validity invariant is maintained by the scoped API or the unsafe
// constructor's contract.
unsafe impl<T: Send, const N: usize, S, C> Send for FanInSpout<T, N, S, C>
where
    S: Spout<T, Error = core::convert::Infallible>,
    C: Collector<T> + Send,
{
}
