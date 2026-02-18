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

mod consumer;
mod producer;

#[cfg(feature = "std")]
pub(crate) mod barrier;
#[cfg(feature = "std")]
mod pool;

pub use consumer::{Consumer, collect};
#[cfg(feature = "std")]
pub use pool::{PoolBuilder, WorkerPool};
pub use producer::Producer;

use alloc::vec::Vec;
use spout::{DropSpout, Spout};

/// Zero-overhead MPSC ring buffer.
///
/// Creates independent producers that each own a [`SpillRing`](crate::SpillRing)
/// running at full speed. No shared state, no contention on the hot path.
pub struct MpscRing<T, const N: usize, S: Spout<T, Error = core::convert::Infallible> = DropSpout> {
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
    #[must_use]
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
    #[must_use]
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
    #[must_use]
    #[cfg(feature = "std")]
    pub fn pool(num_workers: usize) -> PoolBuilder<T, N, DropSpout>
    where
        T: Send + 'static,
    {
        PoolBuilder::new(num_workers)
    }
}

impl<T, const N: usize, S: Spout<T, Error = core::convert::Infallible> + Clone> MpscRing<T, N, S> {
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
    #[allow(clippy::needless_pass_by_value)] // sink is cloned per-producer, consumed by move
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
