//! Fan-in and merger collection APIs for [`WorkerPool`].
//!
//! Scoped (`with_fan_in`, `with_mergers`) and unsafe (`fan_in_unchecked`,
//! `mergers_unchecked`) variants, plus convenience wrappers that bake in
//! [`CollectSpout`].

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::SpillRing;
use spout::{CollectSpout, Spout};

use crate::mpsc::tpc::collector::{Collector, SequencedCollector, UnorderedCollector};
use crate::mpsc::tpc::fan_in::FanInSpout;
use crate::mpsc::tpc::handoff::HandoffSlot;
use crate::mpsc::tpc::merger::MergerHandle;

use super::{SendPtr, WorkerPool};

impl<T, const N: usize, S, F, A> WorkerPool<T, N, S, F, A>
where
    S: Spout<T, Error = core::convert::Infallible>,
    F: Fn(&SpillRing<T, N, S>, usize, &A) + Send + Clone + 'static,
    A: Sync + 'static,
{
    // ---- Fan-in (scoped) ------------------------------------------------

    /// Execute a closure with a [`FanInSpout`] that collects from all
    /// handoff slots.
    ///
    /// The `FanInSpout` cannot escape the closure — its raw pointers to
    /// handoff slots are valid only for the closure's duration. When the
    /// closure returns, the pool regains `&mut self` for subsequent
    /// `dispatch()`/`join()` calls.
    ///
    /// The collector determines how batches are delivered. Use
    /// [`UnorderedCollector`] for simple forwarding, or
    /// [`SequencedCollector`] for ordered delivery.
    ///
    /// # Example
    ///
    /// ```
    /// use spill_ring::{MpscRing, UnorderedCollector};
    /// use spout::CollectSpout;
    ///
    /// let mut pool = MpscRing::<u64, 256>::pool(2)
    ///     .spawn(|ring, _id, count: &u64| {
    ///         for i in 0..*count {
    ///             ring.push(i);
    ///         }
    ///     });
    ///
    /// pool.run(&10);
    ///
    /// pool.with_fan_in(UnorderedCollector::new(CollectSpout::new()), |fan_in| {
    ///     fan_in.flush()
    /// }).unwrap();
    /// ```
    pub fn with_fan_in<C, R>(
        &mut self,
        collector: C,
        f: impl FnOnce(&mut FanInSpout<T, N, S, C>) -> R,
    ) -> R
    where
        C: Collector<T>,
    {
        let slot_ptrs = FanInSpout::<T, N, S, C>::slot_ptrs_from(&self.slots);

        // SAFETY: Slot pointers are valid for the duration of the closure.
        // The pool's &mut self borrow prevents dropping the pool or calling
        // dispatch/join until the closure returns.
        let mut fan_in = unsafe { FanInSpout::new(slot_ptrs, collector) };
        f(&mut fan_in)
    }

    /// Convenience: [`with_fan_in`](Self::with_fan_in) with an
    /// [`UnorderedCollector`] backed by [`CollectSpout`].
    pub fn with_fan_in_collect<R>(
        &mut self,
        f: impl FnOnce(&mut FanInSpout<T, N, S, UnorderedCollector<CollectSpout<T>>>) -> R,
    ) -> R {
        self.with_fan_in(UnorderedCollector::new(CollectSpout::new()), f)
    }

    /// Convenience: [`with_fan_in`](Self::with_fan_in) with a
    /// [`SequencedCollector`] backed by [`CollectSpout`].
    pub fn with_fan_in_collect_sequenced<R>(
        &mut self,
        f: impl FnOnce(&mut FanInSpout<T, N, S, SequencedCollector<T, CollectSpout<T>>>) -> R,
    ) -> R {
        self.with_fan_in(SequencedCollector::from_spout(CollectSpout::new()), f)
    }

    // ---- Fan-in (unsafe) ------------------------------------------------

    /// Create a [`FanInSpout`] with raw pointers to all handoff slots.
    ///
    /// # Safety
    ///
    /// The returned `FanInSpout` must not outlive `self`. The caller must
    /// ensure the pool is not dropped while the `FanInSpout` exists.
    /// Prefer [`with_fan_in()`](Self::with_fan_in) for the safe scoped API.
    pub unsafe fn fan_in_unchecked<C: Collector<T>>(&self, collector: C) -> FanInSpout<T, N, S, C> {
        let slot_ptrs = FanInSpout::<T, N, S, C>::slot_ptrs_from(&self.slots);

        // SAFETY: Caller guarantees the pool outlives the returned FanInSpout.
        unsafe { FanInSpout::new(slot_ptrs, collector) }
    }

    /// Convenience: [`fan_in_unchecked`](Self::fan_in_unchecked) with an
    /// [`UnorderedCollector`] backed by [`CollectSpout`].
    ///
    /// # Safety
    ///
    /// Same as [`fan_in_unchecked`](Self::fan_in_unchecked).
    pub unsafe fn fan_in_collect_unchecked(
        &self,
    ) -> FanInSpout<T, N, S, UnorderedCollector<CollectSpout<T>>> {
        unsafe { self.fan_in_unchecked(UnorderedCollector::new(CollectSpout::new())) }
    }

    /// Convenience: [`fan_in_unchecked`](Self::fan_in_unchecked) with a
    /// [`SequencedCollector`] backed by [`CollectSpout`].
    ///
    /// # Safety
    ///
    /// Same as [`fan_in_unchecked`](Self::fan_in_unchecked).
    pub unsafe fn fan_in_collect_sequenced_unchecked(
        &self,
    ) -> FanInSpout<T, N, S, SequencedCollector<T, CollectSpout<T>>> {
        unsafe { self.fan_in_unchecked(SequencedCollector::from_spout(CollectSpout::new())) }
    }

    // ---- Mergers (scoped) -----------------------------------------------

    /// Execute a closure with M [`MergerHandle`]s, each owning a subset
    /// of handoff slots partitioned round-robin.
    ///
    /// Merger `i` gets slots `{i, i+M, i+2M, ...}`. The closure receives
    /// a mutable slice of merger handles that can be sent to scoped threads
    /// via [`std::thread::scope`].
    ///
    /// # Panics
    ///
    /// Panics if `num_mergers` is 0 or greater than the number of workers.
    pub fn with_mergers<C, R>(
        &mut self,
        num_mergers: usize,
        collector_factory: impl Fn(usize) -> C,
        f: impl FnOnce(&mut [MergerHandle<T, N, S, C>]) -> R,
    ) -> R
    where
        C: Collector<T>,
    {
        assert!(num_mergers > 0, "need at least one merger");
        assert!(
            num_mergers <= self.num_workers,
            "more mergers ({num_mergers}) than workers ({})",
            self.num_workers
        );

        let mut mergers: Vec<MergerHandle<T, N, S, C>> = (0..num_mergers)
            .map(|merger_id| {
                // Round-robin: merger i gets slots {i, i+M, i+2M, ...}
                let slot_ptrs: Box<[SendPtr<HandoffSlot<T, N, S>>]> = self
                    .slots
                    .iter()
                    .enumerate()
                    .filter(|&(w, _)| w % num_mergers == merger_id)
                    .map(|(_, slot)| SendPtr(core::ptr::from_ref(slot)))
                    .collect::<Vec<_>>()
                    .into_boxed_slice();

                let collector = collector_factory(merger_id);
                // SAFETY: Slot pointers valid for the closure duration.
                let fan_in = unsafe { FanInSpout::new(slot_ptrs, collector) };
                MergerHandle::new(fan_in, merger_id)
            })
            .collect();

        f(&mut mergers)
    }

    /// Convenience: [`with_mergers`](Self::with_mergers) with
    /// [`UnorderedCollector`] backed by [`CollectSpout`].
    ///
    /// # Panics
    ///
    /// Panics if `num_mergers` is 0 or greater than the number of workers.
    pub fn with_mergers_collect<R>(
        &mut self,
        num_mergers: usize,
        f: impl FnOnce(&mut [MergerHandle<T, N, S, UnorderedCollector<CollectSpout<T>>>]) -> R,
    ) -> R {
        self.with_mergers(
            num_mergers,
            |_| UnorderedCollector::new(CollectSpout::new()),
            f,
        )
    }

    /// Convenience: [`with_mergers`](Self::with_mergers) with
    /// [`SequencedCollector`] backed by [`CollectSpout`].
    ///
    /// # Panics
    ///
    /// Panics if `num_mergers` is 0 or greater than the number of workers.
    pub fn with_mergers_collect_sequenced<R>(
        &mut self,
        num_mergers: usize,
        f: impl FnOnce(&mut [MergerHandle<T, N, S, SequencedCollector<T, CollectSpout<T>>>]) -> R,
    ) -> R {
        self.with_mergers(
            num_mergers,
            |_| SequencedCollector::from_spout(CollectSpout::new()),
            f,
        )
    }

    // ---- Mergers (unsafe) -----------------------------------------------

    /// Create M [`MergerHandle`]s with raw pointers to handoff slot subsets.
    ///
    /// # Safety
    ///
    /// Returned `MergerHandle`s must not outlive `self`. The caller must
    /// ensure the pool is not dropped while any `MergerHandle` exists.
    /// Prefer [`with_mergers()`](Self::with_mergers) for the safe scoped API.
    ///
    /// # Panics
    ///
    /// Panics if `num_mergers` is 0 or greater than the number of workers.
    pub unsafe fn mergers_unchecked<C>(
        &self,
        num_mergers: usize,
        collector_factory: impl Fn(usize) -> C,
    ) -> Vec<MergerHandle<T, N, S, C>>
    where
        C: Collector<T>,
    {
        assert!(num_mergers > 0, "need at least one merger");
        assert!(
            num_mergers <= self.num_workers,
            "more mergers ({num_mergers}) than workers ({})",
            self.num_workers
        );

        (0..num_mergers)
            .map(|merger_id| {
                let slot_ptrs: Box<[SendPtr<HandoffSlot<T, N, S>>]> = self
                    .slots
                    .iter()
                    .enumerate()
                    .filter(|&(w, _)| w % num_mergers == merger_id)
                    .map(|(_, slot)| SendPtr(core::ptr::from_ref(slot)))
                    .collect::<Vec<_>>()
                    .into_boxed_slice();

                let collector = collector_factory(merger_id);
                // SAFETY: Caller guarantees the pool outlives the MergerHandles.
                let fan_in = unsafe { FanInSpout::new(slot_ptrs, collector) };
                MergerHandle::new(fan_in, merger_id)
            })
            .collect()
    }

    /// Convenience: [`mergers_unchecked`](Self::mergers_unchecked) with
    /// [`UnorderedCollector`] backed by [`CollectSpout`].
    ///
    /// # Safety
    ///
    /// Same as [`mergers_unchecked`](Self::mergers_unchecked).
    ///
    /// # Panics
    ///
    /// Panics if `num_mergers` is 0 or greater than the number of workers.
    pub unsafe fn mergers_collect_unchecked(
        &self,
        num_mergers: usize,
    ) -> Vec<MergerHandle<T, N, S, UnorderedCollector<CollectSpout<T>>>> {
        unsafe {
            self.mergers_unchecked(
                num_mergers,
                |_| UnorderedCollector::new(CollectSpout::new()),
            )
        }
    }

    /// Convenience: [`mergers_unchecked`](Self::mergers_unchecked) with
    /// [`SequencedCollector`] backed by [`CollectSpout`].
    ///
    /// # Safety
    ///
    /// Same as [`mergers_unchecked`](Self::mergers_unchecked).
    ///
    /// # Panics
    ///
    /// Panics if `num_mergers` is 0 or greater than the number of workers.
    pub unsafe fn mergers_collect_sequenced_unchecked(
        &self,
        num_mergers: usize,
    ) -> Vec<MergerHandle<T, N, S, SequencedCollector<T, CollectSpout<T>>>> {
        unsafe {
            self.mergers_unchecked(num_mergers, |_| {
                SequencedCollector::from_spout(CollectSpout::new())
            })
        }
    }
}
