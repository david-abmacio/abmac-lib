//! Zero-overhead MPSC (Multiple-Producer, Single-Consumer) ring buffer.
//!
//! Each producer owns an independent [`SpillRing`] running at full speed (~4.6 Gelem/s).
//! No shared state, no locks, no contention on the hot path.
//! Items automatically flush to the configured spout on overflow and when dropped.
//!
//! # Example
//!
//! ```
//! use spill_ring::MpscRing;
//! use spout::{CollectSpout, ProducerSpout};
//! use std::thread;
//!
//! // Each producer gets its own CollectSpout via ProducerSpout factory
//! let spout = ProducerSpout::new(|_id| CollectSpout::<u64>::new());
//! let producers = MpscRing::<u64, 1024, _>::with_spout(4, spout);
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
mod handoff;
#[cfg(feature = "std")]
mod pool;
#[cfg(feature = "std")]
mod sync;

pub use consumer::Consumer;
#[cfg(feature = "std")]
pub use pool::{PoolBuilder, WorkerPanic, WorkerPool};
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
    /// # Panics
    ///
    /// Panics if `num_producers` is 0.
    ///
    /// # Example
    ///
    /// ```
    /// use spill_ring::MpscRing;
    /// use std::thread;
    ///
    /// let producers = MpscRing::<u64, 256>::producers(4);
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
    pub fn producers(num_producers: usize) -> Vec<Producer<T, N>> {
        assert!(num_producers > 0, "must have at least one producer");
        (0..num_producers).map(|_| Producer::new()).collect()
    }

    /// Create producers with a consumer for manual draining.
    ///
    /// Use this when you need to collect items after producers finish,
    /// rather than auto-flushing to a spout.
    ///
    /// # Panics
    ///
    /// Panics if `num_producers` is 0.
    ///
    /// # Example
    ///
    /// ```
    /// use spill_ring::MpscRing;
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
    /// consumer.collect(finished);
    ///
    /// let mut spout = CollectSpout::new();
    /// consumer.drain(&mut spout);
    /// ```
    #[must_use]
    pub fn with_consumer(num_producers: usize) -> (Vec<Producer<T, N>>, Consumer<T, N>) {
        assert!(num_producers > 0, "must have at least one producer");
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
    /// Create producers with a shared spout for handling evictions.
    ///
    /// Each producer gets a clone of the spout. Items overflow to the spout
    /// during pushes and remaining items flush on drop.
    ///
    /// # Panics
    ///
    /// Panics if `num_producers` is 0.
    ///
    /// # Example
    ///
    /// ```
    /// use spill_ring::MpscRing;
    /// use spout::{CollectSpout, ProducerSpout};
    ///
    /// // Each producer gets its own CollectSpout
    /// let spout = ProducerSpout::new(|_id| CollectSpout::<u64>::new());
    /// let producers = MpscRing::<u64, 64, _>::with_spout(4, spout);
    ///
    /// for producer in producers {
    ///     producer.push(42);
    ///     // Items flush to spout on drop
    /// }
    /// ```
    #[allow(clippy::needless_pass_by_value)] // spout is cloned per-producer, consumed by move
    pub fn with_spout(num_producers: usize, spout: S) -> Vec<Producer<T, N, S>> {
        assert!(num_producers > 0, "must have at least one producer");
        (0..num_producers)
            .map(|_| Producer::with_spout(spout.clone()))
            .collect()
    }

    /// Create a pool builder with a custom spout for persistent worker threads.
    ///
    /// This is the recommended API for maximum performance. Each thread owns
    /// its own pre-warmed ring with a clone of the spout. Call
    /// [`spawn()`](PoolBuilder::spawn) to provide the work function and start
    /// the pool.
    #[cfg(feature = "std")]
    pub fn pool_with_spout(num_workers: usize, spout: S) -> PoolBuilder<T, N, S>
    where
        T: Send + 'static,
        S: Send + 'static,
    {
        PoolBuilder::with_spout(num_workers, spout)
    }
}
