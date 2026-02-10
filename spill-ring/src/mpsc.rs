//! Zero-overhead MPSC (Multiple-Producer, Single-Consumer) ring buffer.
//!
//! Each producer owns an independent [`SpillRing`] running at full speed (~4.6 Gelem/s).
//! No shared state, no locks, no contention on the hot path.
//! Items automatically flush to the configured sink on overflow and when dropped.
//!
//! # Example
//!
//! ```
//! use spill_ring::MpscRing;
//! use spout::{CollectSpout, ProducerSpout};
//! use std::thread;
//!
//! // Each producer gets its own CollectSpout via ProducerSpout factory
//! let sink = ProducerSpout::new(|_id| CollectSpout::<u64>::new());
//! let producers = MpscRing::<u64, 1024, _>::with_sink(4, sink);
//!
//! // Each producer runs at full speed on its own thread
//! thread::scope(|s| {
//!     for producer in producers {
//!         s.spawn(move || {
//!             for i in 0..10_000 {
//!                 producer.push(i);
//!             }
//!             // Items flush to spout when producer drops
//!         });
//!     }
//! });
//! ```

extern crate alloc;

use crate::SpillRing;

use alloc::vec::Vec;
use spout::{DropSpout, Spout};

/// A producer handle for an MPSC ring.
///
/// Each producer owns its own [`SpillRing`] with zero contention.
/// When dropped, remaining items stay in the ring for the consumer to drain.
pub struct Producer<T, const N: usize, S: Spout<T> = DropSpout> {
    ring: SpillRing<T, N, S>,
}

impl<T, const N: usize, S: Spout<T>> Producer<T, N, S> {
    /// Push an item to this producer's ring.
    ///
    /// This is the hot path - runs at ~4.6 Gelem/s.
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

impl<T, const N: usize> Producer<T, N, DropSpout> {
    fn new() -> Self {
        Self {
            ring: SpillRing::new(),
        }
    }
}

impl<T, const N: usize, S: Spout<T>> Producer<T, N, S> {
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
pub struct Consumer<T, const N: usize, S: Spout<T> = DropSpout> {
    rings: Vec<SpillRing<T, N, S>>,
}

impl<T, const N: usize, S: Spout<T>> Consumer<T, N, S> {
    fn new() -> Self {
        Self { rings: Vec::new() }
    }

    fn add_ring(&mut self, ring: SpillRing<T, N, S>) {
        self.rings.push(ring);
    }

    /// Drain all items from all rings into a sink.
    ///
    /// Items are drained in producer order, then FIFO within each producer.
    pub fn drain<Spout2: Spout<T>>(&mut self, sink: &mut Spout2) {
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

/// Zero-overhead MPSC ring buffer.
///
/// Creates independent producers that each own a [`SpillRing`] running at full speed.
/// No shared state, no contention on the hot path.
pub struct MpscRing<T, const N: usize, S: Spout<T> = DropSpout> {
    _marker: core::marker::PhantomData<(T, S)>,
}

impl<T, const N: usize> MpscRing<T, N, DropSpout> {
    /// Create producers with default `DropSpout` (items dropped on overflow).
    ///
    /// Each producer owns its own ring running at full speed.
    /// Items are dropped on overflow and when the producer is dropped.
    ///
    /// # Example
    ///
    /// ```
    /// use spill_ring::MpscRing;
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
    /// use spill_ring::{MpscRing, collect};
    /// use spout::CollectSpout;
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
    /// let mut sink = CollectSpout::new();
    /// consumer.drain(&mut sink);
    /// ```
    pub fn with_consumer(num_producers: usize) -> (Vec<Producer<T, N>>, Consumer<T, N>) {
        let producers = (0..num_producers).map(|_| Producer::new()).collect();
        (producers, Consumer::new())
    }

    /// Create a pool builder for persistent worker threads.
    ///
    /// This is the recommended API for maximum performance. Each thread owns
    /// its own pre-warmed ring. Call [`spawn()`](PoolBuilder::spawn) to provide
    /// the work function and start the pool.
    ///
    /// # Example
    ///
    /// ```
    /// use spill_ring::MpscRing;
    ///
    /// let mut pool = MpscRing::<u64, 1024>::pool(4)
    ///     .spawn(|ring, worker_id, count: &u64| {
    ///         for i in 0..*count {
    ///             ring.push(worker_id as u64 * 1000 + i);
    ///         }
    ///     });
    ///
    /// pool.run(&10_000);
    /// let consumer = pool.into_consumer();
    /// ```
    #[cfg(feature = "std")]
    pub fn pool(num_workers: usize) -> PoolBuilder<T, N, DropSpout>
    where
        T: Send + 'static,
    {
        PoolBuilder::new(num_workers)
    }
}

impl<T, const N: usize, S: Spout<T> + Clone> MpscRing<T, N, S> {
    /// Create producers with a shared sink for handling evictions.
    ///
    /// Each producer gets a clone of the sink. Items overflow to the sink
    /// during pushes and remaining items flush on drop.
    ///
    /// # Example
    ///
    /// ```
    /// use spill_ring::MpscRing;
    /// use spout::{CollectSpout, ProducerSpout};
    ///
    /// // Each producer gets its own CollectSpout
    /// let sink = ProducerSpout::new(|_id| CollectSpout::<u64>::new());
    /// let producers = MpscRing::<u64, 64, _>::with_sink(4, sink);
    ///
    /// for producer in producers {
    ///     producer.push(42);
    ///     // Items flush to spout on drop
    /// }
    /// ```
    pub fn with_sink(num_producers: usize, sink: S) -> Vec<Producer<T, N, S>> {
        (0..num_producers)
            .map(|_| Producer::with_sink(sink.clone()))
            .collect()
    }

    /// Create a pool builder with a custom sink for persistent worker threads.
    ///
    /// This is the recommended API for maximum performance. Each thread owns
    /// its own pre-warmed ring with a clone of the sink. Call
    /// [`spawn()`](PoolBuilder::spawn) to provide the work function and start
    /// the pool.
    #[cfg(feature = "std")]
    pub fn pool_with_sink(num_workers: usize, sink: S) -> PoolBuilder<T, N, S>
    where
        T: Send + 'static,
        S: Send + 'static,
    {
        PoolBuilder::with_sink(num_workers, sink)
    }
}

/// Collect producers back into a consumer for draining.
///
/// This is a helper to reunite producers with their consumer after threads complete.
pub fn collect<T, const N: usize, S: Spout<T>>(
    producers: impl IntoIterator<Item = Producer<T, N, S>>,
    consumer: &mut Consumer<T, N, S>,
) {
    for producer in producers {
        consumer.add_ring(producer.into_ring());
    }
}

#[cfg(feature = "std")]
mod worker_pool {
    use super::{Consumer, DropSpout, SpillRing, Spout, Vec};
    use core::marker::PhantomData;
    use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;

    /// Spin-barrier for low-latency thread synchronization.
    ///
    /// Unlike `std::sync::Barrier` (Mutex + Condvar), this spins on atomics
    /// with no OS syscalls. Eliminates the ~58µs overhead at 8 threads that
    /// dominates short work units.
    struct SpinBarrier {
        count: AtomicUsize,
        generation: AtomicUsize,
        num_threads: usize,
        /// Spin iterations before yielding. Scaled to hardware at construction:
        /// when threads <= available cores, spin longer (no contention for CPU);
        /// when oversubscribed, yield immediately to avoid starvation.
        spin_limit: u32,
    }

    impl SpinBarrier {
        fn new(num_threads: usize) -> Self {
            let cores = thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1);
            // Oversubscribed: yield immediately. Otherwise spin proportional
            // to headroom — more spare cores means less risk of starving.
            let spin_limit = if num_threads > cores {
                0
            } else {
                // 32 spins per spare core, capped at 256. Each PAUSE is ~5ns
                // on x86, so 256 spins ≈ 1.3µs — well under a scheduler tick.
                ((cores - num_threads + 1) as u32 * 32).min(256)
            };

            Self {
                count: AtomicUsize::new(0),
                generation: AtomicUsize::new(0),
                num_threads,
                spin_limit,
            }
        }

        fn wait(&self) {
            let epoch = self.generation.load(Ordering::Relaxed);

            if self.count.fetch_add(1, Ordering::AcqRel) + 1 == self.num_threads {
                // Last thread to arrive — reset count and advance generation.
                self.count.store(0, Ordering::Relaxed);
                self.generation
                    .store(epoch.wrapping_add(1), Ordering::Release);
            } else {
                let mut spins = 0u32;
                while self.generation.load(Ordering::Acquire) == epoch {
                    if spins < self.spin_limit {
                        std::hint::spin_loop();
                        spins += 1;
                    } else {
                        thread::yield_now();
                    }
                }
            }
        }
    }

    /// Builder for constructing a [`WorkerPool`].
    ///
    /// Created via [`MpscRing::pool()`] or [`MpscRing::pool_with_sink()`].
    /// Call [`spawn()`](PoolBuilder::spawn) to provide the work function and
    /// start the pool.
    pub struct PoolBuilder<T, const N: usize, S: Spout<T> = DropSpout> {
        num_workers: usize,
        sink: S,
        _marker: PhantomData<T>,
    }

    impl<T: Send + 'static, const N: usize> PoolBuilder<T, N, DropSpout> {
        pub(super) fn new(num_workers: usize) -> Self {
            assert!(num_workers > 0, "must have at least one worker");
            Self {
                num_workers,
                sink: DropSpout,
                _marker: PhantomData,
            }
        }
    }

    impl<T: Send + 'static, const N: usize, S: Spout<T> + Clone + Send + 'static> PoolBuilder<T, N, S> {
        pub(super) fn with_sink(num_workers: usize, sink: S) -> Self {
            assert!(num_workers > 0, "must have at least one worker");
            Self {
                num_workers,
                sink,
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
            WorkerPool::start(self.num_workers, self.sink, work)
        }
    }

    /// A pool of persistent threads, each owning a pre-warmed [`SpillRing`].
    ///
    /// Thread-per-core design: each thread owns its ring and executes work
    /// locally. No data crosses thread boundaries on the hot path. The only
    /// synchronization is barrier wake-ups between invocations.
    ///
    /// The work function `F` is monomorphized into each thread at spawn time.
    /// Per-invocation arguments `A` are passed by shared reference via atomic
    /// pointer — no boxing, no cloning, no channels.
    pub struct WorkerPool<T, const N: usize, S, F, A>
    where
        S: Spout<T>,
        F: Fn(&SpillRing<T, N, S>, usize, &A) + Send + Clone + 'static,
        A: Sync + 'static,
    {
        num_workers: usize,
        handles: Vec<Option<thread::JoinHandle<SpillRing<T, N, S>>>>,
        args_ptr: Arc<AtomicPtr<A>>,
        shutdown: Arc<AtomicBool>,
        start_barrier: Arc<SpinBarrier>,
        done_barrier: Arc<SpinBarrier>,
        _marker: PhantomData<F>,
    }

    impl<T, const N: usize, S, F, A> WorkerPool<T, N, S, F, A>
    where
        T: Send + 'static,
        S: Spout<T> + Clone + Send + 'static,
        F: Fn(&SpillRing<T, N, S>, usize, &A) + Send + Clone + 'static,
        A: Sync + 'static,
    {
        fn start(num_workers: usize, sink: S, work: F) -> Self {
            let ready_barrier = Arc::new(SpinBarrier::new(num_workers + 1));
            let start_barrier = Arc::new(SpinBarrier::new(num_workers + 1));
            let done_barrier = Arc::new(SpinBarrier::new(num_workers + 1));
            let shutdown = Arc::new(AtomicBool::new(false));
            let args_ptr: Arc<AtomicPtr<A>> = Arc::new(AtomicPtr::new(core::ptr::null_mut()));

            let handles: Vec<_> = (0..num_workers)
                .map(|worker_id| {
                    let sink = sink.clone();
                    let work = work.clone();
                    let ready = Arc::clone(&ready_barrier);
                    let start = Arc::clone(&start_barrier);
                    let done = Arc::clone(&done_barrier);
                    let shutdown = Arc::clone(&shutdown);
                    let args_ptr = Arc::clone(&args_ptr);

                    Some(thread::spawn(move || {
                        // Ring is warm by default (SpillRing::with_sink calls warm())
                        let ring = SpillRing::with_sink(sink);

                        // Signal ready
                        ready.wait();

                        loop {
                            start.wait();

                            if shutdown.load(Ordering::Relaxed) {
                                break;
                            }

                            // Safety: main thread sets args_ptr before triggering
                            // start barrier, and args outlives run() which waits
                            // on done barrier before returning.
                            let args = unsafe { &*args_ptr.load(Ordering::Acquire) };
                            work(&ring, worker_id, args);

                            done.wait();
                        }

                        ring
                    }))
                })
                .collect();

            // Wait for all threads to be spawned and warmed
            ready_barrier.wait();

            Self {
                num_workers,
                handles,
                args_ptr,
                shutdown,
                start_barrier,
                done_barrier,
                _marker: PhantomData,
            }
        }
    }

    impl<T, const N: usize, S, F, A> WorkerPool<T, N, S, F, A>
    where
        S: Spout<T>,
        F: Fn(&SpillRing<T, N, S>, usize, &A) + Send + Clone + 'static,
        A: Sync + 'static,
    {
        /// Get the number of workers in the pool.
        #[inline]
        pub fn num_rings(&self) -> usize {
            self.num_workers
        }

        /// Run the work function on all workers with the given arguments.
        ///
        /// Each worker receives a shared reference to `args`. Blocks until
        /// all workers complete. Takes `&mut self` to prevent overlapping
        /// invocations, which would deadlock on the internal barriers.
        #[inline]
        pub fn run(&mut self, args: &A) {
            // Set args pointer before triggering start barrier
            self.args_ptr
                .store(args as *const A as *mut A, Ordering::Release);
            self.start_barrier.wait();
            self.done_barrier.wait();
        }

        /// Convert the pool into a [`Consumer`] for draining all rings.
        ///
        /// Signals shutdown, joins all threads, and collects their rings.
        pub fn into_consumer(mut self) -> Consumer<T, N, S> {
            let mut consumer = Consumer::new();
            self.shutdown.store(true, Ordering::Relaxed);
            self.start_barrier.wait(); // unblock workers so they see shutdown
            for handle in &mut self.handles {
                if let Some(h) = handle.take() {
                    consumer.add_ring(h.join().unwrap());
                }
            }
            consumer
        }
    }

    impl<T, const N: usize, S, F, A> Drop for WorkerPool<T, N, S, F, A>
    where
        S: Spout<T>,
        F: Fn(&SpillRing<T, N, S>, usize, &A) + Send + Clone + 'static,
        A: Sync + 'static,
    {
        fn drop(&mut self) {
            // Only trigger shutdown if threads are still alive
            if self.handles.iter().any(|h| h.is_some()) {
                self.shutdown.store(true, Ordering::Relaxed);
                self.start_barrier.wait();
                for handle in &mut self.handles {
                    if let Some(h) = handle.take() {
                        let _ = h.join();
                    }
                }
            }
        }
    }
}

#[cfg(feature = "std")]
pub use worker_pool::{PoolBuilder, WorkerPool};

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use spout::CollectSpout;
    use std::sync::{Arc, Mutex};
    use std::vec;

    #[test]
    fn basic_mpsc() {
        let collected = Arc::new(Mutex::new(CollectSpout::new()));
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
        let collected = Arc::new(Mutex::new(CollectSpout::new()));

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
        // With DropSpout (default), items just get dropped
        let producers = MpscRing::<u64, 8>::new(4);
        assert_eq!(producers.len(), 4);
        for p in &producers {
            assert!(p.is_empty());
        }
    }

    #[test]
    fn single_producer() {
        let collected = Arc::new(Mutex::new(CollectSpout::new()));
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
            let mut pool = MpscRing::<u64, 64>::pool(2).spawn(|ring, _id, count: &u64| {
                for i in 0..*count {
                    ring.push(i);
                }
            });

            pool.run(&50);

            let mut consumer = pool.into_consumer();
            let mut sink = CollectSpout::new();
            consumer.drain(&mut sink);

            let items = sink.into_items();
            assert_eq!(items.len(), 100); // 2 workers * 50 each
        }

        #[test]
        fn worker_pool_overflow() {
            // Small ring to force overflow
            let mut pool = MpscRing::<u64, 8>::pool(1).spawn(|ring, _id, count: &u64| {
                for i in 0..*count {
                    ring.push(i);
                }
            });

            pool.run(&100);

            let mut consumer = pool.into_consumer();
            let mut sink = CollectSpout::new();
            consumer.drain(&mut sink);

            // Only last 8 items should remain in ring
            let items = sink.into_items();
            assert_eq!(items.len(), 8);
        }

        #[test]
        fn worker_pool_num_rings() {
            let pool = MpscRing::<u64, 64>::pool(7).spawn(|_ring, _id, _args: &()| {});
            assert_eq!(pool.num_rings(), 7);
        }

        #[test]
        fn worker_pool_empty() {
            let pool = MpscRing::<u64, 64>::pool(4).spawn(|_ring, _id, _args: &()| {});
            let consumer = pool.into_consumer();
            assert!(consumer.is_empty());
            assert_eq!(consumer.num_producers(), 4);
        }

        #[test]
        fn worker_pool_multiple_run_calls() {
            let mut pool = MpscRing::<u64, 128>::pool(2).spawn(|ring, _id, count: &u64| {
                for i in 0..*count {
                    ring.push(i);
                }
            });

            pool.run(&10);
            pool.run(&10);

            let mut consumer = pool.into_consumer();
            let mut sink = CollectSpout::new();
            consumer.drain(&mut sink);

            // Should have 40 items total (2 rings × 20 items each)
            let items = sink.into_items();
            assert_eq!(items.len(), 40);
        }

        #[test]
        fn worker_pool_worker_ids() {
            // Each worker should receive a unique ID from 0..num_workers
            let mut pool = MpscRing::<u64, 64>::pool(4).spawn(|ring, id, _args: &()| {
                // Push worker ID so we can verify from the consumer side
                ring.push(id as u64);
            });

            pool.run(&());

            let mut consumer = pool.into_consumer();
            let mut sink = CollectSpout::new();
            consumer.drain(&mut sink);

            let mut ids = sink.into_items();
            ids.sort();
            assert_eq!(ids, vec![0, 1, 2, 3]);
        }

        #[test]
        fn worker_pool_different_args_per_run() {
            let mut pool = MpscRing::<u64, 128>::pool(1).spawn(|ring, _id, val: &u64| {
                ring.push(*val);
            });

            pool.run(&42);
            pool.run(&99);

            let mut consumer = pool.into_consumer();
            let mut sink = CollectSpout::new();
            consumer.drain(&mut sink);

            let items = sink.into_items();
            assert_eq!(items, vec![42, 99]);
        }

        #[test]
        fn worker_pool_with_sink() {
            let collected = Arc::new(Mutex::new(CollectSpout::new()));

            let mut pool = MpscRing::<u64, 4, _>::pool_with_sink(2, collected.clone()).spawn(
                |ring, _id, count: &u64| {
                    for i in 0..*count {
                        ring.push(i);
                    }
                },
            );

            // Push 10 items per worker into ring of size 4 — forces overflow to sink
            pool.run(&10);

            // Overflow items went to the CollectSpout
            let overflowed = collected.lock().unwrap().items().len();
            assert!(overflowed > 0);

            let mut consumer = pool.into_consumer();
            let mut drain_sink = CollectSpout::new();
            consumer.drain(&mut drain_sink);

            // Remaining items in rings
            let drained = drain_sink.into_items().len();

            // Total: overflowed + drained = 20 (2 workers × 10 each)
            assert_eq!(overflowed + drained, 20);
        }

        #[test]
        fn worker_pool_drop_without_consume() {
            // Pool should drop cleanly without calling into_consumer
            let mut pool = MpscRing::<u64, 64>::pool(4).spawn(|ring, _id, count: &u64| {
                for i in 0..*count {
                    ring.push(i);
                }
            });

            pool.run(&100);
            drop(pool); // Should not panic or hang
        }
    }
}
