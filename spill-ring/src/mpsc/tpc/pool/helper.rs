//! Streaming guard construction for [`WorkerPool`].
//!
//! Generic `streaming_fan_in`/`streaming_mergers` plus convenience wrappers
//! that bake in [`CollectSpout`] with [`UnorderedCollector`] or
//! [`SequencedCollector`].

use crate::SpillRing;
use spout::{CollectSpout, Spout};

use crate::mpsc::tpc::collector::{Collector, SequencedCollector, UnorderedCollector};
use crate::mpsc::tpc::streaming::{StreamingFanIn, StreamingMergers};

use super::WorkerPool;

impl<T, const N: usize, S, F, A> WorkerPool<T, N, S, F, A>
where
    S: Spout<T, Error = core::convert::Infallible>,
    F: Fn(&SpillRing<T, N, S>, usize, &A) + Send + Clone + 'static,
    A: Sync + 'static,
{
    // ---- Streaming fan-in -----------------------------------------------

    /// Create a [`StreamingFanIn`] guard for safe dispatch-flush loops.
    ///
    /// The guard borrows the pool and exposes both `dispatch()`/`join()`
    /// and `flush()` through a single owned value, eliminating the need
    /// for `unsafe` [`fan_in_unchecked()`](Self::fan_in_unchecked).
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
    pub fn streaming_fan_in<C: Collector<T>>(
        &mut self,
        collector: C,
    ) -> StreamingFanIn<'_, T, N, S, F, A, C> {
        StreamingFanIn::new(self, collector)
    }

    /// Convenience: [`streaming_fan_in`](Self::streaming_fan_in) with an
    /// [`UnorderedCollector`] backed by [`CollectSpout`].
    pub fn streaming_fan_in_collect(
        &mut self,
    ) -> StreamingFanIn<'_, T, N, S, F, A, UnorderedCollector<CollectSpout<T>>> {
        self.streaming_fan_in(UnorderedCollector::new(CollectSpout::new()))
    }

    /// Convenience: [`streaming_fan_in`](Self::streaming_fan_in) with a
    /// [`SequencedCollector`] backed by [`CollectSpout`].
    pub fn streaming_fan_in_collect_sequenced(
        &mut self,
    ) -> StreamingFanIn<'_, T, N, S, F, A, SequencedCollector<T, CollectSpout<T>>> {
        self.streaming_fan_in(SequencedCollector::from_spout(CollectSpout::new()))
    }

    // ---- Streaming mergers ----------------------------------------------

    /// Create a [`StreamingMergers`] guard for safe dispatch-flush loops
    /// with multiple parallel mergers.
    ///
    /// The guard borrows the pool and exposes both `dispatch()`/`join()`
    /// and merger access through a single owned value, eliminating the
    /// need for `unsafe` [`mergers_unchecked()`](Self::mergers_unchecked).
    ///
    /// Merger `i` gets slots `{i, i+M, i+2M, ...}` (round-robin).
    /// The `collector_factory` receives the merger index.
    ///
    /// # Panics
    ///
    /// Panics if `num_mergers` is 0 or greater than the number of workers.
    pub fn streaming_mergers<C: Collector<T>>(
        &mut self,
        num_mergers: usize,
        collector_factory: impl Fn(usize) -> C,
    ) -> StreamingMergers<'_, T, N, S, F, A, C> {
        StreamingMergers::new(self, num_mergers, collector_factory)
    }

    /// Convenience: [`streaming_mergers`](Self::streaming_mergers) with
    /// [`UnorderedCollector`] backed by [`CollectSpout`].
    ///
    /// # Panics
    ///
    /// Panics if `num_mergers` is 0 or greater than the number of workers.
    pub fn streaming_mergers_collect(
        &mut self,
        num_mergers: usize,
    ) -> StreamingMergers<'_, T, N, S, F, A, UnorderedCollector<CollectSpout<T>>> {
        self.streaming_mergers(
            num_mergers,
            |_| UnorderedCollector::new(CollectSpout::new()),
        )
    }

    /// Convenience: [`streaming_mergers`](Self::streaming_mergers) with
    /// [`SequencedCollector`] backed by [`CollectSpout`].
    ///
    /// # Panics
    ///
    /// Panics if `num_mergers` is 0 or greater than the number of workers.
    pub fn streaming_mergers_collect_sequenced(
        &mut self,
        num_mergers: usize,
    ) -> StreamingMergers<'_, T, N, S, F, A, SequencedCollector<T, CollectSpout<T>>> {
        self.streaming_mergers(num_mergers, |_| {
            SequencedCollector::from_spout(CollectSpout::new())
        })
    }
}
