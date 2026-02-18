//! Composable fan-in spout for collecting from TPC handoff slots.
//!
//! `FanInSpout` iterates N handoff slots on `flush()`, drains any
//! published rings into an inner spout, and recycles the empty rings
//! back to workers. Created via [`WorkerPool::with_fan_in()`] (scoped,
//! safe) or [`WorkerPool::fan_in_unchecked()`] (unsafe, unscoped).

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;
use spout::Spout;

use super::handoff::HandoffSlot;
use super::pool::SendPtr;

/// Collects from N handoff slots, emits to a downstream spout.
///
/// On [`flush()`](Spout::flush), iterates all assigned handoff slots,
/// takes any published batch (Acquire), drains items into the inner
/// spout via [`send_all()`](Spout::send_all), then recycles the empty
/// ring back to the worker (Release).
///
/// [`send()`](Spout::send) is pass-through to the inner spout — for
/// items that bypass handoff (e.g., metadata, control signals). The
/// primary data path is `flush()`.
///
/// Created via [`WorkerPool::with_fan_in()`] (scoped, safe) or
/// [`WorkerPool::fan_in_unchecked()`] (unsafe, unscoped).
pub struct FanInSpout<
    T,
    const N: usize,
    S: Spout<T, Error = core::convert::Infallible>,
    Out: Spout<T>,
> {
    /// Raw pointers to handoff slots owned by WorkerPool.
    /// Validity invariant: these point into the pool's `Box<[HandoffSlot]>`.
    /// The scoped API ensures the pool outlives this struct.
    slots: Box<[SendPtr<HandoffSlot<T, N, S>>]>,
    /// Downstream spout receiving collected items.
    inner: Out,
}

impl<T, const N: usize, S, Out> FanInSpout<T, N, S, Out>
where
    S: Spout<T, Error = core::convert::Infallible>,
    Out: Spout<T>,
{
    /// Create a FanInSpout from raw slot pointers and an inner spout.
    ///
    /// # Safety
    ///
    /// All pointers in `slots` must remain valid for the lifetime of
    /// this struct. The scoped API ([`WorkerPool::with_fan_in()`])
    /// guarantees this; the unsafe constructor
    /// ([`WorkerPool::fan_in_unchecked()`]) relies on the caller.
    pub(crate) unsafe fn new(slots: Box<[SendPtr<HandoffSlot<T, N, S>>]>, inner: Out) -> Self {
        Self { slots, inner }
    }

    /// Reference to the inner spout.
    #[inline]
    pub fn inner(&self) -> &Out {
        &self.inner
    }

    /// Mutable reference to the inner spout.
    #[inline]
    pub fn inner_mut(&mut self) -> &mut Out {
        &mut self.inner
    }

    /// Consume and return the inner spout.
    #[inline]
    pub fn into_inner(self) -> Out {
        self.inner
    }

    /// Number of handoff slots this fan-in collects from.
    #[inline]
    #[must_use]
    pub fn num_slots(&self) -> usize {
        self.slots.len()
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

impl<T, const N: usize, S, Out> Spout<T> for FanInSpout<T, N, S, Out>
where
    S: Spout<T, Error = core::convert::Infallible>,
    Out: Spout<T>,
{
    type Error = Out::Error;

    /// Pass-through: forward an externally-produced item to the inner spout.
    ///
    /// This is NOT the primary data path — [`flush()`](Spout::flush) is.
    #[inline]
    fn send(&mut self, item: T) -> Result<(), Self::Error> {
        self.inner.send(item)
    }

    /// Collect all available batches from handoff slots, drain into inner.
    ///
    /// For each slot with a published batch:
    /// 1. Swap null into batch pointer (Acquire) — takes ownership of ring
    /// 2. Drain all items from ring into inner spout via `send_all()`
    /// 3. Offer empty ring back to worker via recycle pointer (Release)
    ///
    /// Slots where the worker hasn't published yet are skipped silently.
    fn flush(&mut self) -> Result<(), Self::Error> {
        for slot_ptr in self.slots.iter() {
            // SAFETY: Pointer validity guaranteed by scoped API (with_fan_in)
            // or caller contract (fan_in_unchecked).
            let slot = unsafe { &*slot_ptr.0 };
            if let Some(mut ring) = slot.collect() {
                self.inner.send_all(ring.drain())?;
                slot.offer_recycle(ring);
            }
        }
        self.inner.flush()
    }
}

// SAFETY: FanInSpout is Send when Out is Send. The slot pointers are
// SendPtr (manually Send). The validity invariant is maintained by
// the scoped API or the unsafe constructor's contract.
unsafe impl<T, const N: usize, S, Out> Send for FanInSpout<T, N, S, Out>
where
    S: Spout<T, Error = core::convert::Infallible>,
    Out: Spout<T> + Send,
{
}
