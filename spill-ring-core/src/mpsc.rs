//! Zero-overhead MPSC (Multiple-Producer, Single-Consumer) ring buffer.
//!
//! Each producer owns an independent [`SpillRing`] running at full speed (~4.6 Gelem/s
//! with `no-atomics`). No shared state, no locks, no contention on the hot path.
//! Items automatically flush to the configured sink on overflow and when dropped.
//!
//! # Example
//!
//! ```
//! use spill_ring_core::{MpscRing, CollectSink, ProducerSink};
//! use std::thread;
//!
//! // Each producer gets its own CollectSink via ProducerSink factory
//! let sink = ProducerSink::new(|_id| CollectSink::<u64>::new());
//! let producers = MpscRing::<u64, 1024, _>::with_sink(4, sink);
//!
//! // Each producer runs at full speed on its own thread
//! thread::scope(|s| {
//!     for producer in producers {
//!         s.spawn(move || {
//!             for i in 0..10_000 {
//!                 producer.push(i);
//!             }
//!             // Items flush to sink when producer drops
//!         });
//!     }
//! });
//! ```

extern crate alloc;

use crate::{DropSink, Sink, SpillRing};
use alloc::vec::Vec;

/// A producer handle for an MPSC ring.
///
/// Each producer owns its own [`SpillRing`] with zero contention.
/// When dropped, remaining items stay in the ring for the consumer to drain.
pub struct Producer<T, const N: usize, S: Sink<T> = DropSink> {
    ring: SpillRing<T, N, S>,
}

impl<T, const N: usize, S: Sink<T>> Producer<T, N, S> {
    /// Push an item to this producer's ring.
    ///
    /// This is the hot path - runs at ~4.6 Gelem/s with `no-atomics`.
    #[inline]
    pub fn push(&self, item: T) {
        self.ring.push(item);
    }

    /// Check if the ring is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }

    /// Check if the ring is full.
    #[inline]
    pub fn is_full(&self) -> bool {
        self.ring.is_full()
    }

    /// Get the number of items in the ring.
    #[inline]
    pub fn len(&self) -> usize {
        self.ring.len()
    }

    /// Get the ring's capacity.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.ring.capacity()
    }

    fn into_ring(self) -> SpillRing<T, N, S> {
        self.ring
    }
}

impl<T, const N: usize> Producer<T, N, DropSink> {
    fn new() -> Self {
        Self {
            ring: SpillRing::new(),
        }
    }
}

impl<T, const N: usize, S: Sink<T>> Producer<T, N, S> {
    fn with_sink(sink: S) -> Self {
        Self {
            ring: SpillRing::with_sink(sink),
        }
    }
}

/// Consumer handle for an MPSC ring.
///
/// Collects producer rings and provides drain functionality.
/// Use this when you need manual control over when items are consumed.
pub struct Consumer<T, const N: usize, S: Sink<T> = DropSink> {
    rings: Vec<SpillRing<T, N, S>>,
}

impl<T, const N: usize, S: Sink<T>> Consumer<T, N, S> {
    fn new() -> Self {
        Self { rings: Vec::new() }
    }

    fn add_ring(&mut self, ring: SpillRing<T, N, S>) {
        self.rings.push(ring);
    }

    /// Drain all items from all rings into a sink.
    ///
    /// Items are drained in producer order, then FIFO within each producer.
    pub fn drain<Sink2: Sink<T>>(&mut self, sink: &mut Sink2) {
        for ring in &mut self.rings {
            sink.send_all(ring.drain());
        }
        sink.flush();
    }

    /// Get the number of producers/rings.
    pub fn num_producers(&self) -> usize {
        self.rings.len()
    }

    /// Check if all rings are empty.
    pub fn is_empty(&self) -> bool {
        self.rings.iter().all(|r| r.is_empty())
    }

    /// Get total items across all rings.
    pub fn len(&self) -> usize {
        self.rings.iter().map(|r| r.len()).sum()
    }
}

/// Collect producers back into a consumer for draining.
pub fn collect<T, const N: usize, S: Sink<T>>(
    producers: impl IntoIterator<Item = Producer<T, N, S>>,
    consumer: &mut Consumer<T, N, S>,
) {
    for producer in producers {
        consumer.add_ring(producer.into_ring());
    }
}

/// Zero-overhead MPSC ring buffer.
///
/// Creates independent producers that each own a [`SpillRing`] running at full speed.
/// No shared state, no contention on the hot path.
pub struct MpscRing<T, const N: usize, S: Sink<T> = DropSink> {
    _marker: core::marker::PhantomData<(T, S)>,
}

impl<T, const N: usize> MpscRing<T, N, DropSink> {
    /// Create producers with default `DropSink` (items dropped on overflow).
    ///
    /// Each producer owns its own ring running at full speed.
    /// Items are dropped on overflow and when the producer is dropped.
    ///
    /// # Example
    ///
    /// ```
    /// use spill_ring_core::MpscRing;
    /// use std::thread;
    ///
    /// let producers = MpscRing::<u64, 256>::new(4);
    ///
    /// thread::scope(|s| {
    ///     for (id, producer) in producers.into_iter().enumerate() {
    ///         s.spawn(move || {
    ///             for i in 0..1000 {
    ///                 producer.push(id as u64 * 10000 + i);
    ///             }
    ///             // Producer drops here, remaining items dropped
    ///         });
    ///     }
    /// });
    /// ```
    #[allow(clippy::new_ret_no_self)]
    pub fn new(num_producers: usize) -> Vec<Producer<T, N>> {
        (0..num_producers).map(|_| Producer::new()).collect()
    }

    /// Create producers with a consumer for manual draining.
    ///
    /// Use this when you need to collect items after producers finish,
    /// rather than auto-flushing to a sink.
    ///
    /// # Example
    ///
    /// ```
    /// use spill_ring_core::{MpscRing, CollectSink, collect};
    /// use std::thread;
    ///
    /// let (producers, mut consumer) = MpscRing::<u64, 256>::with_consumer(4);
    ///
    /// let finished: Vec<_> = thread::scope(|s| {
    ///     producers.into_iter().map(|producer| {
    ///         s.spawn(move || {
    ///             for i in 0..1000 {
    ///                 producer.push(i);
    ///             }
    ///             producer
    ///         })
    ///     }).collect::<Vec<_>>()
    ///     .into_iter()
    ///     .map(|h| h.join().unwrap())
    ///     .collect()
    /// });
    ///
    /// collect(finished, &mut consumer);
    ///
    /// let mut sink = CollectSink::new();
    /// consumer.drain(&mut sink);
    /// ```
    pub fn with_consumer(num_producers: usize) -> (Vec<Producer<T, N>>, Consumer<T, N>) {
        let producers = (0..num_producers).map(|_| Producer::new()).collect();
        (producers, Consumer::new())
    }

    /// Create a pre-warmed worker pool with persistent threads.
    ///
    /// This is the recommended API for maximum performance. Each thread owns
    /// its own ring, cache is pre-warmed, and threads are ready before this
    /// returns.
    ///
    /// # Example
    ///
    /// ```
    /// use spill_ring_core::MpscRing;
    ///
    /// let pool = MpscRing::<u64, 1024>::pooled(4);
    /// pool.run(10_000); // Each worker pushes 10k items
    /// let consumer = pool.into_consumer();
    /// ```
    #[cfg(feature = "std")]
    pub fn pooled(num_workers: usize) -> WorkerPool<T, N, DropSink>
    where
        T: Send + 'static,
    {
        WorkerPool::new(num_workers)
    }
}

impl<T, const N: usize, S: Sink<T> + Clone> MpscRing<T, N, S> {
    /// Create producers with a shared sink for handling evictions.
    ///
    /// Each producer gets a clone of the sink. Items overflow to the sink
    /// during pushes and remaining items flush on drop.
    ///
    /// # Example
    ///
    /// ```
    /// use spill_ring_core::{MpscRing, CollectSink, ProducerSink};
    ///
    /// // Each producer gets its own CollectSink
    /// let sink = ProducerSink::new(|_id| CollectSink::<u64>::new());
    /// let producers = MpscRing::<u64, 64, _>::with_sink(4, sink);
    ///
    /// for producer in producers {
    ///     producer.push(42);
    ///     // Items flush to sink on drop
    /// }
    /// ```
    pub fn with_sink(num_producers: usize, sink: S) -> Vec<Producer<T, N, S>> {
        (0..num_producers)
            .map(|_| Producer::with_sink(sink.clone()))
            .collect();
        let consumer = Consumer::new();
        (producers, consumer)
    }

    /// Create a pre-warmed worker pool with a custom sink.
    ///
    /// This is the recommended API for maximum performance. Each thread owns
    /// its own ring with a clone of the sink, cache is pre-warmed, and threads
    /// are ready before this returns.
    #[cfg(feature = "std")]
    pub fn pooled_with_sink(num_workers: usize, sink: S) -> WorkerPool<T, N, S>
    where
        T: Send + 'static,
        S: Send + 'static,
    {
        WorkerPool::with_sink(num_workers, sink)
    }
}

/// Collect producers back into a consumer for draining.
///
/// This is a helper to reunite producers with their consumer after threads complete.
pub fn collect_producers<T, const N: usize, S: Sink<T>>(
    producers: impl IntoIterator<Item = Producer<T, N, S>>,
    consumer: &mut Consumer<T, N, S>,
) {
    for producer in producers {
        consumer.add_ring(producer.into_ring());
    }
}

#[cfg(feature = "std")]
mod worker_pool {
    use super::{Consumer, DropSink, Sink, SpillRing, Vec};
    use core::mem::MaybeUninit;
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::sync::{Arc, Barrier};
    use std::thread;

    /// A pool of persistent threads with pre-warmed [`SpillRing`]s.
    ///
    /// Each thread owns its ring for the entire lifetime of the pool.
    /// Threads are spawned once at pool creation and kept alive until dropped.
    ///
    /// The pool is ready to use immediately after construction - all threads
    /// are spawned and cache-warmed before `new()` returns.
    ///
    /// This is a convenience API for simple use cases. For maximum performance,
    /// use the pattern from `mpsc_prewarmed` in the benchmarks which keeps the
    /// work loop in user code where the compiler can fully optimize it.
    pub struct WorkerPool<T, const N: usize, S: Sink<T> = DropSink> {
        cmd_txs: Vec<Sender<u64>>,
        handles: Vec<Option<thread::JoinHandle<SpillRing<T, N, S>>>>,
        start_barrier: Arc<Barrier>,
        done_barrier: Arc<Barrier>,
    }

    impl<T: Send + 'static, const N: usize> WorkerPool<T, N, DropSink> {
        /// Create a new pool with the specified number of persistent threads.
        ///
        /// All threads are spawned and cache-warmed before this returns.
        pub fn new(num_workers: usize) -> Self {
            Self::with_sink(num_workers, DropSink)
        }
    }

    impl<T: Send + 'static, const N: usize, S: Sink<T> + Clone + Send + 'static> WorkerPool<T, N, S> {
        /// Create a new pool with a custom sink for evictions.
        ///
        /// All threads are spawned and cache-warmed before this returns.
        pub fn with_sink(num_workers: usize, sink: S) -> Self {
            assert!(num_workers > 0, "must have at least one worker");

            let ready_barrier = Arc::new(Barrier::new(num_workers + 1));
            let start_barrier = Arc::new(Barrier::new(num_workers + 1));
            let done_barrier = Arc::new(Barrier::new(num_workers + 1));

            let (cmd_txs, cmd_rxs): (Vec<Sender<u64>>, Vec<Receiver<u64>>) =
                (0..num_workers).map(|_| channel()).unzip();

            let handles: Vec<_> = cmd_rxs
                .into_iter()
                .map(|rx| {
                    let sink = sink.clone();
                    let ready = Arc::clone(&ready_barrier);
                    let start = Arc::clone(&start_barrier);
                    let done = Arc::clone(&done_barrier);

                    Some(thread::spawn(move || {
                        let ring = SpillRing::with_sink(sink);

                        // Warm cache by touching all slots with uninitialized writes.
                        // This brings the ring's memory into L1/L2 cache without
                        // requiring T: Default or constructing real values.
                        for i in 0..N {
                            unsafe {
                                let slot = &ring.buffer[i];
                                // Write uninitialized memory to touch the cache line
                                core::ptr::write_volatile(
                                    (*slot.data.get()).as_mut_ptr(),
                                    MaybeUninit::<T>::uninit().assume_init_read(),
                                );
                            }
                        }
                        // Reset indices (no items actually in ring)
                        ring.head.store(0);
                        ring.tail.store(0);

                        // Signal ready, then wait for work
                        ready.wait();

                        while let Ok(count) = rx.recv() {
                            start.wait();
                            for _ in 0..count {
                                unsafe {
                                    // Push uninitialized values - this is a synthetic
                                    // benchmark helper, real usage should push real data
                                    ring.push(MaybeUninit::<T>::uninit().assume_init_read());
                                }
                            }
                            done.wait();
                        }
                        ring
                    }))
                })
                .collect();

            // Wait for all threads to be warmed and ready
            ready_barrier.wait();

            Self {
                cmd_txs,
                handles,
                start_barrier,
                done_barrier,
            }
        }
    }

    impl<T: Send + 'static, const N: usize, S: Sink<T> + Send + 'static> WorkerPool<T, N, S> {
        /// Get the number of workers in the pool.
        #[inline]
        pub fn num_rings(&self) -> usize {
            self.handles.len()
        }

        /// Run work on all rings in parallel.
        ///
        /// Each worker pushes `count` items to its ring. Returns after all complete.
        #[inline]
        pub fn run(&self, count: u64) {
            for tx in &self.cmd_txs {
                let _ = tx.send(count);
            }
            self.start_barrier.wait();
            self.done_barrier.wait();
        }

        /// Send work count to all workers without waiting.
        ///
        /// Call `wait_start()` then `wait_done()` to synchronize.
        #[inline]
        pub fn send(&self, count: u64) {
            for tx in &self.cmd_txs {
                let _ = tx.send(count);
            }
        }

        /// Wait for all workers to start.
        #[inline]
        pub fn wait_start(&self) {
            self.start_barrier.wait();
        }

        /// Wait for all workers to complete.
        #[inline]
        pub fn wait_done(&self) {
            self.done_barrier.wait();
        }

        /// Convert the pool into a [`Consumer`] for draining.
        pub fn into_consumer(mut self) -> Consumer<T, N, S> {
            let mut consumer = Consumer::new();
            self.cmd_txs.clear();
            for handle in &mut self.handles {
                if let Some(h) = handle.take() {
                    consumer.add_ring(h.join().unwrap());
                }
            }
            consumer
        }
    }

    impl<T, const N: usize, S: Sink<T>> Drop for WorkerPool<T, N, S> {
        fn drop(&mut self) {
            self.cmd_txs.clear();
            for handle in &mut self.handles {
                if let Some(h) = handle.take() {
                    let _ = h.join();
                }
            }
        }
    }
}

#[cfg(feature = "std")]
pub use worker_pool::WorkerPool;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CollectSink;
    use std::sync::{Arc, Mutex};

    #[test]
    fn basic_mpsc() {
        let collected = Arc::new(Mutex::new(CollectSink::new()));
        let producers = MpscRing::<u64, 8, _>::with_sink(2, collected.clone());

        // Producer 0
        producers[0].push(1);
        producers[0].push(2);

        // Producer 1
        producers[1].push(10);
        producers[1].push(20);

        // Drop producers to flush to sink
        drop(producers);

        let items = collected.lock().unwrap().items().to_vec();
        assert_eq!(items.len(), 4);
        assert!(items.contains(&1));
        assert!(items.contains(&2));
        assert!(items.contains(&10));
        assert!(items.contains(&20));
    }

    #[test]
    fn producer_overflow_to_sink() {
        let collected = Arc::new(Mutex::new(CollectSink::new()));

        let producers = MpscRing::<u64, 4, _>::with_sink(1, collected.clone());
        let producer = producers.into_iter().next().unwrap();

        // Overflow - push 10 items into ring of size 4
        for i in 0..10 {
            producer.push(i);
        }

        // First 6 items should have been evicted to sink
        assert_eq!(collected.lock().unwrap().items().len(), 6);

        // Drop flushes remaining 4
        drop(producer);
        assert_eq!(collected.lock().unwrap().items().len(), 10);
    }

    #[test]
    fn empty_producers_drop_sink() {
        // With DropSink (default), items just get dropped
        let producers = MpscRing::<u64, 8>::new(4);
        assert_eq!(producers.len(), 4);
        for p in &producers {
            assert!(p.is_empty());
        }
    }

    #[test]
    fn single_producer() {
        let collected = Arc::new(Mutex::new(CollectSink::new()));
        let producers = MpscRing::<u64, 16, _>::with_sink(1, collected.clone());

        let producer = producers.into_iter().next().unwrap();
        for i in 0..10 {
            producer.push(i);
        }

        drop(producer);

        let items = collected.lock().unwrap().items().to_vec();
        assert_eq!(items, vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
    }

    #[cfg(feature = "std")]
    mod worker_pool_tests {
        use super::*;

        #[test]
        fn basic_worker_pool() {
            let pool = WorkerPool::<u64, 64>::new(2);

            pool.run(50);

            let mut consumer = pool.into_consumer();
            let mut sink = CollectSink::new();
            consumer.drain(&mut sink);

            let items = sink.into_items();
            assert_eq!(items.len(), 100); // 2 workers * 50 each
        }

        #[test]
        fn worker_pool_overflow() {
            // Small ring to force overflow
            let pool = WorkerPool::<u64, 8>::new(1);

            pool.run(100);

            let mut consumer = pool.into_consumer();
            let mut sink = CollectSink::new();
            consumer.drain(&mut sink);

            // Only last 8 items should remain in ring
            let items = sink.into_items();
            assert_eq!(items.len(), 8);
        }

        #[test]
        fn worker_pool_num_rings() {
            let pool = WorkerPool::<u64, 64>::new(7);
            assert_eq!(pool.num_rings(), 7);
        }

        #[test]
        fn worker_pool_empty() {
            let pool = WorkerPool::<u64, 64>::new(4);
            let consumer = pool.into_consumer();
            assert!(consumer.is_empty());
            assert_eq!(consumer.num_producers(), 4);
        }

        #[test]
        fn worker_pool_multiple_run_calls() {
            let pool = WorkerPool::<u64, 128>::new(2);

            pool.run(10);
            pool.run(10);

            let mut consumer = pool.into_consumer();
            let mut sink = CollectSink::new();
            consumer.drain(&mut sink);

            // Should have 40 items total (2 rings × 20 items each)
            let items = sink.into_items();
            assert_eq!(items.len(), 40);
        }
    }
}
