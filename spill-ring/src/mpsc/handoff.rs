//! Per-worker handoff slot and signaling for the TPC WorkerPool.
//!
//! Each worker has one `HandoffSlot` (for ring transfer) and one
//! `WorkerSignal` (for go/done/shutdown coordination). Both are
//! cache-line aligned to prevent false sharing.

extern crate alloc;

use alloc::boxed::Box;
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};

use crate::SpillRing;
use spout::Spout;

use super::sync;

/// Per-producer handoff slot. Cache-line aligned to prevent false sharing.
///
/// Two atomic pointers form a double-buffered channel between a single
/// producer thread and the consumer (main thread):
///
/// - `batch`: producer publishes a full ring here; consumer collects it.
/// - `recycle`: consumer returns an empty ring here; producer reuses it.
///
/// Each pointer is either null (empty) or a valid `Box<SpillRing>` that
/// was leaked via `Box::into_raw`. Ownership transfers on every swap.
#[repr(C, align(128))]
pub(crate) struct HandoffSlot<T, const N: usize, S: Spout<T, Error = core::convert::Infallible>> {
    /// Published ring, or null. Producer -> Consumer.
    batch: AtomicPtr<SpillRing<T, N, S>>,
    /// Recycled empty ring, or null. Consumer -> Producer.
    recycle: AtomicPtr<SpillRing<T, N, S>>,
    /// Batch sequence number. Stamped by main thread before dispatch,
    /// read by consumer during collect(). Not accessed by workers.
    batch_seq: AtomicU64,
}

impl<T, const N: usize, S: Spout<T, Error = core::convert::Infallible>> HandoffSlot<T, N, S> {
    /// Create a new empty handoff slot (both pointers null).
    pub(crate) fn new() -> Self {
        Self {
            batch: AtomicPtr::new(ptr::null_mut()),
            recycle: AtomicPtr::new(ptr::null_mut()),
            batch_seq: AtomicU64::new(0),
        }
    }

    /// Producer: publish a ring into the batch slot.
    ///
    /// Returns the previous batch pointer (null if consumer already collected,
    /// non-null if the previous batch was uncollected — caller must handle it).
    #[inline]
    pub(crate) fn publish(&self, ring: Box<SpillRing<T, N, S>>) -> *mut SpillRing<T, N, S> {
        let ptr = Box::into_raw(ring);
        // Ordering: Release — publishes ring contents before consumer reads the pointer.
        self.batch.swap(ptr, Ordering::Release)
    }

    /// Consumer: collect the published ring, if any.
    ///
    /// Returns `Some(ring)` if the producer published, `None` if the slot was empty.
    #[inline]
    pub(crate) fn collect(&self) -> Option<Box<SpillRing<T, N, S>>> {
        // Ordering: Acquire — pairs with producer's Release to see ring contents.
        let ptr = self.batch.swap(ptr::null_mut(), Ordering::Acquire);
        if ptr.is_null() {
            None
        } else {
            // SAFETY: Non-null pointer was created by `Box::into_raw` in `publish()`.
            // Ownership transfers to us via the atomic swap (we stored null, so no
            // other thread will read this pointer again).
            Some(unsafe { Box::from_raw(ptr) })
        }
    }

    /// Consumer: offer a recycled (empty) ring back to the producer.
    ///
    /// If a previous recycled ring was not yet taken by the producer, it is
    /// dropped (the producer was too slow to reclaim it).
    #[inline]
    pub(crate) fn offer_recycle(&self, ring: Box<SpillRing<T, N, S>>) {
        let ptr = Box::into_raw(ring);
        // Ordering: Release — publishes the recycled ring before producer reads.
        let old = self.recycle.swap(ptr, Ordering::Release);
        if !old.is_null() {
            // SAFETY: Previous recycled ring was never reclaimed. We own it.
            drop(unsafe { Box::from_raw(old) });
        }
    }

    /// Producer: take a recycled ring, if available.
    ///
    /// Returns `Some(ring)` if the consumer offered one, `None` otherwise.
    #[inline]
    pub(crate) fn take_recycle(&self) -> Option<Box<SpillRing<T, N, S>>> {
        // Ordering: Acquire — pairs with consumer's Release to see recycled ring.
        let ptr = self.recycle.swap(ptr::null_mut(), Ordering::Acquire);
        if ptr.is_null() {
            None
        } else {
            // SAFETY: Non-null pointer was created by `Box::into_raw` in `offer_recycle()`.
            // Ownership transfers to us via the atomic swap.
            Some(unsafe { Box::from_raw(ptr) })
        }
    }

    /// Main thread: stamp this slot's batch sequence number before dispatch.
    #[inline]
    pub(crate) fn stamp_seq(&self, seq: u64) {
        // Ordering: Release — publishes seq before go signal wakes worker.
        // The go signal's Release provides the cross-thread fence; this
        // Release ensures the seq value is visible when the consumer later
        // reads the batch with Acquire.
        self.batch_seq.store(seq, Ordering::Release);
    }

    /// Consumer: read the batch sequence number for the last published batch.
    ///
    /// Call after `collect()` returns `Some` — the Acquire on `batch.swap(null)`
    /// transitively sees the seq stored by the main thread.
    #[inline]
    pub(crate) fn read_seq(&self) -> u64 {
        // Ordering: Acquire — pairs with main thread's Release store.
        // The happens-before chain: main Release(seq) → main Release(go) →
        // worker Acquire(go) → worker Release(publish) → consumer Acquire(batch).
        // This Acquire is technically redundant (the batch Acquire already
        // provides the fence), but makes the dependency explicit.
        self.batch_seq.load(Ordering::Acquire)
    }
}

impl<T, const N: usize, S: Spout<T, Error = core::convert::Infallible>> Drop
    for HandoffSlot<T, N, S>
{
    fn drop(&mut self) {
        // Ordering: get_mut() on drop — exclusive access, no ordering needed.
        let batch = *self.batch.get_mut();
        if !batch.is_null() {
            // SAFETY: Non-null pointer was created by `Box::into_raw`.
            drop(unsafe { Box::from_raw(batch) });
        }
        let recycle = *self.recycle.get_mut();
        if !recycle.is_null() {
            // SAFETY: Non-null pointer was created by `Box::into_raw`.
            drop(unsafe { Box::from_raw(recycle) });
        }
    }
}

/// Per-worker control signals. Cache-line aligned to prevent false sharing.
///
/// Three atomic booleans coordinate between the main thread and one worker:
///
/// - `go`: main sets to wake the worker for a new batch.
/// - `done`: worker sets when work is complete and batch is published.
/// - `shutdown`: main sets to tell the worker to exit after its next wake.
///
/// The go/done pair forms a Release/Acquire edge that synchronizes access
/// to `args_ptr` and the handoff slot. Shutdown piggybacks on the go
/// signal's fence (always read after go is observed).
#[repr(C, align(128))]
pub(crate) struct WorkerSignal {
    /// Main -> Worker: "start working".
    go: AtomicBool,
    /// Worker -> Main: "work complete, batch published".
    done: AtomicBool,
    /// Main -> Worker: "shut down after next wake".
    shutdown: AtomicBool,
}

impl WorkerSignal {
    /// Create a new signal set (all false).
    pub(crate) fn new() -> Self {
        Self {
            go: AtomicBool::new(false),
            done: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
        }
    }

    /// Main thread: signal the worker to start working.
    ///
    /// Must be called after storing `args_ptr` with Release ordering.
    #[inline]
    pub(crate) fn signal_go(&self) {
        // Ordering: Release — publishes args_ptr store before worker wakes.
        self.go.store(true, Ordering::Release);
    }

    /// Worker: block until the go signal is set, using adaptive spin-then-yield.
    #[inline]
    pub(crate) fn wait_for_go(&self, spin_limit: u32) {
        sync::spin_wait(spin_limit, || {
            // Ordering: Acquire — pairs with main's Release to see args_ptr.
            self.go.load(Ordering::Acquire)
        });
    }

    /// Worker: clear the go signal after waking.
    #[inline]
    pub(crate) fn clear_go(&self) {
        // Ordering: Relaxed — only the owning worker writes this after wake.
        // The Acquire load in wait_for_go already established the fence.
        self.go.store(false, Ordering::Relaxed);
    }

    /// Worker: signal that work is complete and the batch is published.
    #[inline]
    pub(crate) fn signal_done(&self) {
        // Ordering: Release — publishes handoff slot batch before main reads.
        self.done.store(true, Ordering::Release);
    }

    /// Main thread: check if the worker has completed.
    #[inline]
    pub(crate) fn is_done(&self) -> bool {
        // Ordering: Acquire — pairs with worker's Release to see published batch.
        self.done.load(Ordering::Acquire)
    }

    /// Main thread: clear the done flag for the next round.
    #[inline]
    pub(crate) fn clear_done(&self) {
        // Ordering: Relaxed — only the main thread writes this between rounds.
        // No cross-thread data depends on this store.
        self.done.store(false, Ordering::Relaxed);
    }

    /// Main thread: signal the worker to shut down.
    #[inline]
    pub(crate) fn signal_shutdown(&self) {
        // Ordering: Relaxed — piggybacks on the go signal's Release/Acquire fence.
        // Worker always reads shutdown after observing go=true (Acquire).
        self.shutdown.store(true, Ordering::Relaxed);
    }

    /// Worker: check if shutdown has been requested.
    #[inline]
    pub(crate) fn is_shutdown(&self) -> bool {
        // Ordering: Relaxed — piggybacks on the go signal's Acquire fence.
        // This is always read after wait_for_go() returns, which provides
        // the necessary Acquire ordering.
        self.shutdown.load(Ordering::Relaxed)
    }
}
