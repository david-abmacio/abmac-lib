//! TPC (Thread-Per-Core) WorkerPool with per-worker signaling and
//! double-buffered handoff slots.
//!
//! Each worker thread owns a `SpillRing` and publishes completed batches
//! via an `AtomicPtr`-based handoff slot. The main thread coordinates
//! via per-worker go/done/shutdown signals — no shared barriers.

mod collection;
mod helper;

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::marker::PhantomData;
use core::sync::atomic::{AtomicPtr, Ordering};
use std::thread;

use crate::SpillRing;
use spout::{DropSpout, Spout};

use super::super::Consumer;
use super::handoff::{HandoffSlot, WorkerSignal};
use super::sync;

/// Wrapper to send a raw pointer across thread boundaries.
///
/// # Safety
///
/// The pointed-to data must outlive all threads that receive this pointer.
/// `WorkerPool` guarantees this: `shutdown_and_join` joins all threads
/// before the `Box` allocations backing these pointers are dropped.
pub(crate) struct SendPtr<T>(pub(crate) *const T);

// SAFETY: The pointer targets heap-allocated data (`Box<[...]>`, `Box<AtomicPtr>`)
// owned by `WorkerPool`. All worker threads are joined in `shutdown_and_join`
// (called from `Drop`) before these allocations are freed.
unsafe impl<T> Send for SendPtr<T> {}

/// Error returned when a worker thread panicked during execution.
#[derive(Debug)]
pub struct WorkerPanic {
    /// Index of the worker that panicked.
    pub worker_id: usize,
}

impl core::fmt::Display for WorkerPanic {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "worker {} panicked", self.worker_id)
    }
}

impl std::error::Error for WorkerPanic {}

/// Builder for constructing a [`WorkerPool`].
///
/// Created via [`MpscRing::pool()`](super::super::MpscRing::pool) or
/// [`MpscRing::pool_with_spout()`](super::super::MpscRing::pool_with_spout).
/// Call [`spawn()`](PoolBuilder::spawn) to provide the work function and
/// start the pool.
pub struct PoolBuilder<
    T,
    const N: usize,
    S: Spout<T, Error = core::convert::Infallible> = DropSpout,
> {
    num_workers: usize,
    spout: S,
    _marker: PhantomData<T>,
}

impl<T: Send + 'static, const N: usize> PoolBuilder<T, N, DropSpout> {
    pub(crate) fn new(num_workers: usize) -> Self {
        assert!(num_workers > 0, "must have at least one worker");
        Self {
            num_workers,
            spout: DropSpout,
            _marker: PhantomData,
        }
    }
}

impl<
    T: Send + 'static,
    const N: usize,
    S: Spout<T, Error = core::convert::Infallible> + Clone + Send + 'static,
> PoolBuilder<T, N, S>
{
    pub(crate) fn with_spout(num_workers: usize, spout: S) -> Self {
        assert!(num_workers > 0, "must have at least one worker");
        Self {
            num_workers,
            spout,
            _marker: PhantomData,
        }
    }

    /// Spawn worker threads with the given work function.
    ///
    /// The work function is cloned once per thread at spawn time and
    /// monomorphized — no dynamic dispatch on the hot path. Each worker
    /// owns its own pre-warmed [`SpillRing`].
    ///
    /// All threads are spawned and cache-warmed before this returns.
    ///
    /// # Example
    ///
    /// ```
    /// use spill_ring::MpscRing;
    ///
    /// let mut pool = MpscRing::<u64, 1024>::pool(4)
    ///     .spawn(|ring, worker_id, args: &u64| {
    ///         for i in 0..*args {
    ///             ring.push(worker_id as u64 * 1000 + i);
    ///         }
    ///     });
    ///
    /// pool.run(&100);
    /// let consumer = pool.into_consumer();
    /// ```
    pub fn spawn<F, A>(self, work: F) -> WorkerPool<T, N, S, F, A>
    where
        F: Fn(&SpillRing<T, N, S>, usize, &A) + Send + Clone + 'static,
        A: Sync + 'static,
    {
        WorkerPool::start(self.num_workers, self.spout, work)
    }
}

/// A pool of persistent threads, each owning a pre-warmed [`SpillRing`].
///
/// Thread-per-core design: each thread owns its ring with ZERO contention
/// on the produce path. Completed batches transfer to the consumer via
/// per-worker `AtomicPtr`-based handoff slots. Per-worker go/done/shutdown
/// signals replace shared barriers — a panicked worker is detected via
/// `JoinHandle::is_finished()` instead of causing a deadlock.
///
/// The work function `F` is monomorphized into each thread at spawn time.
/// Per-invocation arguments `A` are passed by shared reference via atomic
/// pointer — no boxing, no cloning, no channels.
pub struct WorkerPool<T, const N: usize, S, F, A>
where
    S: Spout<T, Error = core::convert::Infallible>,
    F: Fn(&SpillRing<T, N, S>, usize, &A) + Send + Clone + 'static,
    A: Sync + 'static,
{
    pub(crate) num_workers: usize,
    handles: Vec<Option<thread::JoinHandle<()>>>,
    /// Heap-allocated handoff slots, one per worker. Stable address for
    /// raw pointers held by worker threads.
    pub(crate) slots: Box<[HandoffSlot<T, N, S>]>,
    /// Heap-allocated worker signals, one per worker. Stable address for
    /// raw pointers held by worker threads.
    signals: Box<[WorkerSignal]>,
    /// Args pointer for the current batch. Heap-allocated for stable address.
    args_ptr: Box<AtomicPtr<A>>,
    /// Pre-computed spin limit for adaptive spin-then-yield polling.
    spin_limit: u32,
    /// Monotonic batch sequence counter, incremented on each dispatch.
    batch_seq: u64,
    _marker: PhantomData<F>,
}

impl<T, const N: usize, S, F, A> WorkerPool<T, N, S, F, A>
where
    T: Send + 'static,
    S: Spout<T, Error = core::convert::Infallible> + Clone + Send + 'static,
    F: Fn(&SpillRing<T, N, S>, usize, &A) + Send + Clone + 'static,
    A: Sync + 'static,
{
    #[allow(clippy::needless_pass_by_value)] // spout is cloned per-worker, consumed by move
    fn start(num_workers: usize, spout: S, work: F) -> Self {
        let spin_limit = sync::compute_spin_limit(num_workers + 1);

        // Allocate signals and slots on the heap for stable addresses.
        let signals: Box<[WorkerSignal]> = (0..num_workers)
            .map(|_| WorkerSignal::new())
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let slots: Box<[HandoffSlot<T, N, S>]> = (0..num_workers)
            .map(|_| HandoffSlot::new())
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let args_ptr = Box::new(AtomicPtr::new(core::ptr::null_mut()));

        // Pre-allocate 2*N rings: one active + one spare per worker.
        // After construction, zero allocations occur in steady state —
        // rings cycle between active/published/recycled via pointer swaps.
        let mut rings: Vec<Box<SpillRing<T, N, S>>> = (0..num_workers * 2)
            .map(|_| Box::new(SpillRing::with_spout(spout.clone())))
            .collect();

        // Place spare rings into recycle slots (one per worker).
        for (i, slot) in slots.iter().enumerate() {
            let spare = rings.pop().expect("pre-allocated 2*N rings");
            slot.offer_recycle(spare);
            // Remaining rings[0..num_workers] are the active rings.
            let _ = i;
        }

        // Raw pointers to heap-allocated data — stable across WorkerPool moves.
        let signals_base: *const WorkerSignal = signals.as_ptr();
        let slots_base: *const HandoffSlot<T, N, S> = slots.as_ptr();
        let args_raw: *const AtomicPtr<A> = &*args_ptr;

        let mut handles = Vec::with_capacity(num_workers);
        for worker_id in 0..num_workers {
            let work = work.clone();
            let active_ring = rings.pop().expect("pre-allocated 2*N rings");

            // SAFETY: signals_base, slots_base, args_raw point into Box
            // allocations that outlive all worker threads (shutdown_and_join
            // joins all threads before dropping the Boxes).
            let signal_ptr = SendPtr(unsafe { signals_base.add(worker_id) });
            let slot_ptr = SendPtr(unsafe { slots_base.add(worker_id) });
            let args_send = SendPtr(args_raw);

            let handle = thread::spawn(move || {
                worker_loop(
                    active_ring,
                    worker_id,
                    work,
                    signal_ptr,
                    slot_ptr,
                    args_send,
                    spin_limit,
                );
            });
            handles.push(Some(handle));
        }
        debug_assert!(
            rings.is_empty(),
            "all pre-allocated rings should be distributed"
        );

        Self {
            num_workers,
            handles,
            slots,
            signals,
            args_ptr,
            spin_limit,
            batch_seq: 0,
            _marker: PhantomData,
        }
    }
}

/// Worker thread entry point. Runs until shutdown is signaled.
///
/// # Safety invariants (maintained by `WorkerPool`):
/// - `signal`, `slot`, `args_ptr` point to heap allocations that outlive this thread.
/// - `args_ptr` contains a valid pointer when `go` is signaled (not shutdown).
fn worker_loop<T, const N: usize, S, F, A>(
    mut ring: Box<SpillRing<T, N, S>>,
    worker_id: usize,
    work: F,
    signal: SendPtr<WorkerSignal>,
    slot: SendPtr<HandoffSlot<T, N, S>>,
    args_ptr: SendPtr<AtomicPtr<A>>,
    spin_limit: u32,
) where
    S: Spout<T, Error = core::convert::Infallible>,
    F: Fn(&SpillRing<T, N, S>, usize, &A),
{
    // SAFETY: Pointers are valid for the lifetime of the worker thread.
    // WorkerPool::shutdown_and_join() joins all threads before dropping
    // the Box allocations that back these pointers.
    let signal = unsafe { &*signal.0 };
    let slot = unsafe { &*slot.0 };
    let args_atom = unsafe { &*args_ptr.0 };

    loop {
        signal.wait_for_go(spin_limit);

        if signal.is_shutdown() {
            // Publish the final ring so the consumer can drain it.
            let old = slot.publish(ring);
            if !old.is_null() {
                // SAFETY: Non-null pointer was created by a previous publish().
                // The consumer didn't collect it before shutdown. Drop it —
                // shutdown_and_join already collected batches before signaling.
                drop(unsafe { Box::from_raw(old) });
            }
            return;
        }

        signal.clear_go();

        // SAFETY: Main thread stores a valid args pointer with Release ordering
        // before signaling go with Release ordering. The Acquire in wait_for_go
        // establishes the happens-before edge. The args reference is valid for
        // the duration of this work unit (main thread blocks until done).
        let args_raw = args_atom.load(Ordering::Acquire);
        assert!(!args_raw.is_null(), "worker {worker_id}: args_ptr is null");
        let args = unsafe { &*args_raw };

        work(&ring, worker_id, args);

        // Take the recycled ring from the consumer. In steady state this is
        // always available (2*N rings pre-allocated, consumer recycles after
        // draining). On the first round the spare from construction is here.
        // If somehow unavailable (consumer hasn't recycled yet), the uncollected
        // batch merge below handles it — the worker keeps its current ring.
        let fresh = match slot.take_recycle() {
            Some(recycled) => recycled,
            None => {
                // Consumer hasn't recycled yet. Skip the swap — keep the
                // current ring. Items accumulate until the next round.
                signal.signal_done();
                continue;
            }
        };

        // Swap the active ring into the handoff slot.
        let published = core::mem::replace(&mut ring, fresh);
        let old = slot.publish(published);

        // If the consumer hasn't collected the previous batch, merge it back.
        if !old.is_null() {
            // SAFETY: The non-null pointer was created by a previous `publish()` call
            // via `Box::into_raw`. We now own it (swapped out of the atomic).
            let mut old_ring = unsafe { Box::from_raw(old) };
            for item in old_ring.drain() {
                ring.push_mut(item);
            }
        }

        signal.signal_done();
    }
}

impl<T, const N: usize, S, F, A> WorkerPool<T, N, S, F, A>
where
    S: Spout<T, Error = core::convert::Infallible>,
    F: Fn(&SpillRing<T, N, S>, usize, &A) + Send + Clone + 'static,
    A: Sync + 'static,
{
    /// Get the number of workers in the pool.
    #[inline]
    #[must_use]
    pub fn num_rings(&self) -> usize {
        self.num_workers
    }

    /// Borrow the handoff slots (for streaming guard construction).
    #[inline]
    pub(crate) fn slots(&self) -> &[HandoffSlot<T, N, S>] {
        &self.slots
    }

    /// Non-blocking: stamp batch sequence, store args, signal all workers.
    ///
    /// Returns immediately. Call [`join()`](Self::join) to wait for
    /// completion, or [`collect()`](Self::collect) to drain available
    /// results without waiting.
    ///
    /// # Panics
    ///
    /// Panics if any worker has previously panicked.
    pub fn dispatch(&mut self, args: &A) {
        self.check_panicked()
            .expect("worker panicked in previous round");

        let seq = self.batch_seq;
        self.batch_seq = self.batch_seq.wrapping_add(1);

        // Stamp batch sequence on all slots before signaling go.
        for slot in self.slots.iter() {
            slot.stamp_seq(seq);
        }

        // Ordering: Release — publishes args data before workers read it.
        self.args_ptr
            .store(core::ptr::from_ref(args).cast_mut(), Ordering::Release);

        for sig in self.signals.iter() {
            sig.signal_go();
        }
    }

    /// Block until all workers signal done. Detects panics.
    ///
    /// After `join()` returns, all handoff slots contain published batches
    /// ready for [`collect()`](Self::collect).
    pub fn join(&mut self) -> Result<(), WorkerPanic> {
        // Poll done signals with adaptive spin-then-yield.
        // Also check for panicked workers to avoid hanging.
        sync::spin_wait(self.spin_limit, || self.all_done_or_panicked());

        // Check if a worker panicked during this round.
        self.check_panicked()?;

        // Reset done flags for the next round.
        for sig in self.signals.iter() {
            sig.clear_done();
        }

        Ok(())
    }

    /// Collect all available batches from handoff slots into a spout.
    ///
    /// Non-blocking: iterates each slot, takes any published batch (Acquire),
    /// drains items into `out`, then recycles the empty ring (Release).
    /// Slots where the worker hasn't published yet are skipped.
    ///
    /// Returns the total number of items drained across all slots.
    ///
    /// Can be called:
    /// - After `dispatch()` but before `join()` (streaming collection)
    /// - After `join()` (all batches available)
    /// - Between `run()` calls (collect previous round's batches)
    ///
    /// # Ring Recycling
    ///
    /// Empty rings are offered back to workers via `offer_recycle()`. If the
    /// worker hasn't taken the previous recycled ring, the old one is dropped.
    /// This only occurs when the consumer collects faster than workers produce
    /// (consumer is not the bottleneck).
    pub fn collect<Out: Spout<T>>(&mut self, out: &mut Out) -> Result<usize, Out::Error> {
        let mut total = 0usize;
        for slot in self.slots.iter() {
            if let Some(mut ring) = slot.collect() {
                let count = ring.len();
                out.send_all(ring.drain())?;
                slot.offer_recycle(ring);
                total += count;
            }
        }
        Ok(total)
    }

    /// Run the work function on all workers with the given arguments.
    ///
    /// Each worker receives a shared reference to `args`. Blocks until
    /// all workers complete. Takes `&mut self` to prevent overlapping
    /// invocations.
    ///
    /// # Panics
    ///
    /// Panics if any worker thread has panicked. Use [`try_run`](Self::try_run)
    /// for graceful error handling.
    #[inline]
    pub fn run(&mut self, args: &A) {
        if let Err(e) = self.try_run(args) {
            panic!("{e}");
        }
    }

    /// Run the work function on all workers, returning an error if any
    /// worker panicked.
    ///
    /// Unlike [`run`](Self::run), this does not panic on worker failure.
    /// After a worker panic, the pool is in a degraded state — further
    /// calls will return `Err` for the same worker.
    pub fn try_run(&mut self, args: &A) -> Result<(), WorkerPanic> {
        self.dispatch(args);
        self.join()
    }

    /// Convert the pool into a [`Consumer`] for draining all rings.
    ///
    /// Signals shutdown, joins all threads, and collects their rings
    /// from the handoff slots.
    #[must_use]
    pub fn into_consumer(mut self) -> Consumer<T, N, S> {
        let rings = self.shutdown_and_join();
        let mut consumer = Consumer::new();
        for ring in rings {
            consumer.add_ring(*ring);
        }
        consumer
    }

    /// Check if all workers are done, or if any has panicked (finished early).
    fn all_done_or_panicked(&self) -> bool {
        let mut all_done = true;
        for (i, sig) in self.signals.iter().enumerate() {
            if !sig.is_done() {
                // Not done — check if the handle is finished (panicked).
                if self.handles[i]
                    .as_ref()
                    .is_none_or(thread::JoinHandle::is_finished)
                {
                    // Worker panicked or was already joined — stop waiting.
                    return true;
                }
                all_done = false;
            }
        }
        all_done
    }

    /// Check if any worker has panicked. Returns Err for the first one found.
    fn check_panicked(&self) -> Result<(), WorkerPanic> {
        for (i, handle) in self.handles.iter().enumerate() {
            if handle.as_ref().is_some_and(thread::JoinHandle::is_finished) {
                // A finished handle that hasn't been joined means it panicked
                // (normal exit only happens via shutdown_and_join).
                if !self.signals[i].is_done() {
                    return Err(WorkerPanic { worker_id: i });
                }
            }
        }
        Ok(())
    }

    /// Signal shutdown, join all threads, and collect remaining rings.
    fn shutdown_and_join(&mut self) -> Vec<Box<SpillRing<T, N, S>>> {
        if !self.handles.iter().any(Option::is_some) {
            return Vec::new();
        }

        // Collect any batches published by the last run() before signaling
        // shutdown — otherwise the worker's shutdown publish would overwrite
        // the last batch.
        let mut rings = Vec::with_capacity(self.num_workers);
        for slot in self.slots.iter() {
            if let Some(ring) = slot.collect() {
                rings.push(ring);
            }
        }

        // Signal shutdown on all workers.
        for sig in self.signals.iter() {
            sig.signal_shutdown();
            sig.signal_go();
        }

        // Join all threads, ignoring panics.
        for handle in &mut self.handles {
            if let Some(h) = handle.take() {
                // Ignoring join errors — the worker may have panicked.
                // Its ring (if published) is still in the handoff slot.
                let _ = h.join();
            }
        }

        // Collect final rings published by workers during shutdown.
        for slot in self.slots.iter() {
            if let Some(ring) = slot.collect() {
                rings.push(ring);
            }
        }
        rings
    }
}

impl<T, const N: usize, S, F, A> Drop for WorkerPool<T, N, S, F, A>
where
    S: Spout<T, Error = core::convert::Infallible>,
    F: Fn(&SpillRing<T, N, S>, usize, &A) + Send + Clone + 'static,
    A: Sync + 'static,
{
    fn drop(&mut self) {
        self.shutdown_and_join();
    }
}
