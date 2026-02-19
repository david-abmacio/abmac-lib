//! Guard types for safe streaming dispatch-flush loops.
//!
//! [`StreamingFanIn`] and [`StreamingMergers`] borrow a [`WorkerPool`]
//! and expose both dispatch/join and flush through a single owned value.
//! When the guard drops, the pool's `&mut self` is released.
//!
//! Created via [`WorkerPool::streaming_fan_in()`] and
//! [`WorkerPool::streaming_mergers()`].

use spout::Spout;

use super::collector::Collector;
use super::fan_in::FanInSpout;
use super::merger::MergerHandle;
use super::pool::{WorkerPanic, WorkerPool};
use crate::SpillRing;

/// Guard for safe streaming dispatch-flush loops with a single collector.
///
/// Borrows `&mut WorkerPool` and holds a [`FanInSpout`] over all handoff
/// slots. Exposes both dispatch/join (delegated to the pool) and flush
/// (delegated to the fan-in) through a single value.
///
/// # Safety (internal)
///
/// The `FanInSpout` holds raw pointers into the pool's `Box<[HandoffSlot]>`.
/// These remain valid because:
/// 1. The guard borrows `&mut self` — pool cannot be dropped or moved
/// 2. `Box<[HandoffSlot]>` has stable heap addresses
/// 3. `dispatch()`/`join()` only touch signals/args/batch_seq, never slots
/// 4. Handoff slots use `AtomicPtr` interior mutability
///
/// # Example
///
/// ```
/// use spill_ring::{MpscRing, SequencedCollector};
/// use spout::CollectSpout;
///
/// let mut pool = MpscRing::<u64, 256>::pool(4)
///     .spawn(|ring, id, round: &u64| {
///         ring.push(*round * 100 + id as u64);
///     });
///
/// let mut stream = pool.streaming_fan_in(
///     SequencedCollector::from_spout(CollectSpout::new()),
/// );
///
/// for round in 0..5u64 {
///     stream.dispatch(&round);
///     stream.join().unwrap();
///     stream.flush().unwrap();
/// }
///
/// let items = stream.into_collector().into_inner_spout().into_items();
/// assert_eq!(items.len(), 20);
/// ```
pub struct StreamingFanIn<'pool, T, const N: usize, S, F, A, C>
where
    S: Spout<T, Error = core::convert::Infallible>,
    F: Fn(&SpillRing<T, N, S>, usize, &A) + Send + Clone + 'static,
    A: Sync + 'static,
    C: Collector<T>,
{
    pool: &'pool mut WorkerPool<T, N, S, F, A>,
    fan_in: FanInSpout<T, N, S, C>,
}

impl<'pool, T, const N: usize, S, F, A, C> StreamingFanIn<'pool, T, N, S, F, A, C>
where
    S: Spout<T, Error = core::convert::Infallible>,
    F: Fn(&SpillRing<T, N, S>, usize, &A) + Send + Clone + 'static,
    A: Sync + 'static,
    C: Collector<T>,
{
    /// Create a new streaming fan-in guard.
    ///
    /// # Safety
    ///
    /// The `FanInSpout` is created from raw pointers to the pool's
    /// handoff slots. The `&'pool mut` borrow ensures the pool outlives
    /// this guard.
    pub(crate) fn new(pool: &'pool mut WorkerPool<T, N, S, F, A>, collector: C) -> Self {
        let slot_ptrs = FanInSpout::<T, N, S, C>::slot_ptrs_from(pool.slots());
        // SAFETY: Pool is borrowed for 'pool lifetime. Slots are heap-allocated
        // with stable addresses. The guard ensures the pool cannot be dropped
        // while the FanInSpout exists.
        let fan_in = unsafe { FanInSpout::new(slot_ptrs, collector) };
        Self { pool, fan_in }
    }

    /// Non-blocking: stamp batch sequence, store args, signal all workers.
    ///
    /// See [`WorkerPool::dispatch()`] for details.
    #[inline]
    pub fn dispatch(&mut self, args: &A) {
        self.pool.dispatch(args);
    }

    /// Block until all workers signal done.
    ///
    /// See [`WorkerPool::join()`] for details.
    #[inline]
    pub fn join(&mut self) -> Result<(), WorkerPanic> {
        self.pool.join()
    }

    /// Dispatch and join in one call.
    ///
    /// See [`WorkerPool::try_run()`] for details.
    #[inline]
    pub fn try_run(&mut self, args: &A) -> Result<(), WorkerPanic> {
        self.pool.try_run(args)
    }

    /// Dispatch and join, panicking on worker failure.
    ///
    /// See [`WorkerPool::run()`] for details.
    #[inline]
    pub fn run(&mut self, args: &A) {
        self.pool.run(args);
    }

    /// Collect all available batches from handoff slots into the collector.
    ///
    /// See [`FanInSpout::flush()`] for details.
    #[inline]
    pub fn flush(&mut self) -> Result<(), C::Error> {
        self.fan_in.flush()
    }

    /// Reference to the collector.
    #[inline]
    pub fn collector(&self) -> &C {
        self.fan_in.collector()
    }

    /// Mutable reference to the collector.
    #[inline]
    pub fn collector_mut(&mut self) -> &mut C {
        self.fan_in.collector_mut()
    }

    /// Consume the guard and return the collector.
    #[inline]
    pub fn into_collector(self) -> C {
        self.fan_in.into_collector()
    }

    /// Number of handoff slots this fan-in collects from.
    #[inline]
    #[must_use]
    pub fn num_slots(&self) -> usize {
        self.fan_in.num_slots()
    }
}

/// Guard for safe streaming dispatch-flush loops with multiple mergers.
///
/// Borrows `&mut WorkerPool` and holds a set of [`MergerHandle`]s,
/// each owning a subset of handoff slots partitioned round-robin.
/// Exposes dispatch/join (delegated to the pool) and merger access
/// through a single value.
///
/// # Example
///
/// ```
/// use spill_ring::{MpscRing, UnorderedCollector};
/// use spout::CollectSpout;
/// use std::thread;
///
/// let mut pool = MpscRing::<u64, 256>::pool(8)
///     .spawn(|ring, id, _: &()| { ring.push(id as u64); });
///
/// let mut stream = pool.streaming_mergers(
///     2,
///     |_| UnorderedCollector::new(CollectSpout::new()),
/// );
///
/// stream.run(&());
///
/// thread::scope(|s| {
///     for merger in stream.mergers().iter_mut() {
///         s.spawn(|| merger.flush().unwrap());
///     }
/// });
/// ```
pub struct StreamingMergers<'pool, T, const N: usize, S, F, A, C>
where
    S: Spout<T, Error = core::convert::Infallible>,
    F: Fn(&SpillRing<T, N, S>, usize, &A) + Send + Clone + 'static,
    A: Sync + 'static,
    C: Collector<T>,
{
    pool: &'pool mut WorkerPool<T, N, S, F, A>,
    mergers: Vec<MergerHandle<T, N, S, C>>,
}

impl<'pool, T, const N: usize, S, F, A, C> StreamingMergers<'pool, T, N, S, F, A, C>
where
    S: Spout<T, Error = core::convert::Infallible>,
    F: Fn(&SpillRing<T, N, S>, usize, &A) + Send + Clone + 'static,
    A: Sync + 'static,
    C: Collector<T>,
{
    /// Create a new streaming mergers guard.
    ///
    /// # Panics
    ///
    /// Panics if `num_mergers` is 0 or greater than the number of workers.
    pub(crate) fn new(
        pool: &'pool mut WorkerPool<T, N, S, F, A>,
        num_mergers: usize,
        collector_factory: impl Fn(usize) -> C,
    ) -> Self {
        assert!(num_mergers > 0, "need at least one merger");
        assert!(
            num_mergers <= pool.num_rings(),
            "more mergers ({num_mergers}) than workers ({})",
            pool.num_rings()
        );

        let mergers = (0..num_mergers)
            .map(|merger_id| {
                use super::pool::SendPtr;
                // Round-robin: merger i gets slots {i, i+M, i+2M, ...}
                let slot_ptrs = pool
                    .slots()
                    .iter()
                    .enumerate()
                    .filter(|&(w, _)| w % num_mergers == merger_id)
                    .map(|(_, slot)| SendPtr(core::ptr::from_ref(slot)))
                    .collect::<Vec<_>>()
                    .into_boxed_slice();

                let collector = collector_factory(merger_id);
                // SAFETY: Pool is borrowed for 'pool lifetime. Slots are
                // heap-allocated with stable addresses.
                let fan_in = unsafe { FanInSpout::new(slot_ptrs, collector) };
                MergerHandle::new(fan_in, merger_id)
            })
            .collect();

        Self { pool, mergers }
    }

    /// Non-blocking: stamp batch sequence, store args, signal all workers.
    #[inline]
    pub fn dispatch(&mut self, args: &A) {
        self.pool.dispatch(args);
    }

    /// Block until all workers signal done.
    #[inline]
    pub fn join(&mut self) -> Result<(), WorkerPanic> {
        self.pool.join()
    }

    /// Dispatch and join in one call.
    #[inline]
    pub fn try_run(&mut self, args: &A) -> Result<(), WorkerPanic> {
        self.pool.try_run(args)
    }

    /// Dispatch and join, panicking on worker failure.
    #[inline]
    pub fn run(&mut self, args: &A) {
        self.pool.run(args);
    }

    /// Mutable access to the merger handles.
    #[inline]
    pub fn mergers(&mut self) -> &mut [MergerHandle<T, N, S, C>] {
        &mut self.mergers
    }

    /// Consume the guard and return the merger handles.
    #[inline]
    pub fn into_mergers(self) -> Vec<MergerHandle<T, N, S, C>> {
        self.mergers
    }
}

// Bring Vec into scope for the alloc-gated module.
extern crate alloc;
use alloc::vec::Vec;
