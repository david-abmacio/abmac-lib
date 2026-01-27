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
            .collect()
    }
}

#[cfg(all(test, feature = "std"))]
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
}
