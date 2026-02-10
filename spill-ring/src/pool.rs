//! Steal-on-full ring pool for high-throughput multi-threaded scenarios.
//!
//! Producers push at full single-ring speed (~4.6 Gelem/s). When a ring fills,
//! the producer swaps it for a pre-fetched spare (lock-free on hot path).
//!
//! # Architecture
//!
//! ```text
//! Producer Thread                                      Drain Thread
//!      │                                                    │
//!      ▼                                                    ▼
//!  ┌────────┐                                         ┌──────────┐
//!  │ active │ push push push (lock-free)              │ Drain to │
//!  └────────┘                                         │  Spout   │
//!      │ (full)                                       └──────────┘
//!      ▼                                                    ▲
//!  swap with spare ◄─────(pre-fetched)                     │
//!      │                                                    │
//!      └──────────► full_queue ─────────────────────────────┘
//! ```
//!
//! Key: No locks on the push hot path. Locking only happens when
//! fetching a new spare ring (background operation).

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

#[cfg(feature = "std")]
use std::sync::{Arc, Condvar, Mutex};

use crate::{DropSpout, SpillRing};

/// A boxed ring for heap allocation (allows moving between threads).
type BoxedRing<T, const N: usize> = Box<SpillRing<T, N, DropSpout>>;

/// Thread-safe queue for full rings waiting to be drained.
#[cfg(feature = "std")]
struct DrainQueue<T, const N: usize> {
    queue: Mutex<VecDeque<BoxedRing<T, N>>>,
    condvar: Condvar,
    shutdown: AtomicBool,
}

#[cfg(feature = "std")]
impl<T, const N: usize> DrainQueue<T, N> {
    fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            condvar: Condvar::new(),
            shutdown: AtomicBool::new(false),
        }
    }

    /// Push a full ring to the drain queue.
    fn push(&self, ring: BoxedRing<T, N>) {
        self.queue.lock().unwrap().push_back(ring);
        self.condvar.notify_one();
    }

    /// Pop a ring to drain (blocks until available or shutdown).
    fn pop(&self) -> Option<BoxedRing<T, N>> {
        let mut queue = self.queue.lock().unwrap();
        loop {
            if let Some(ring) = queue.pop_front() {
                return Some(ring);
            }
            if self.shutdown.load(Ordering::Acquire) {
                // Drain remaining on shutdown
                return queue.pop_front();
            }
            queue = self.condvar.wait(queue).unwrap();
        }
    }

    fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        self.condvar.notify_all();
    }
}

/// Pool of empty rings for producers to steal from.
#[cfg(feature = "std")]
struct EmptyPool<T, const N: usize> {
    rings: Mutex<Vec<BoxedRing<T, N>>>,
}

#[cfg(feature = "std")]
impl<T, const N: usize> EmptyPool<T, N> {
    fn new(num_rings: usize) -> Self {
        let mut rings = Vec::with_capacity(num_rings);
        for _ in 0..num_rings {
            let mut ring = Box::new(SpillRing::new());
            ring.warm();
            rings.push(ring);
        }
        Self {
            rings: Mutex::new(rings),
        }
    }

    /// Steal an empty ring (allocates if pool exhausted).
    fn steal(&self) -> BoxedRing<T, N> {
        self.rings.lock().unwrap().pop().unwrap_or_else(|| {
            let mut ring = Box::new(SpillRing::new());
            ring.warm();
            ring
        })
    }

    /// Return an empty ring to the pool.
    fn return_ring(&self, ring: BoxedRing<T, N>) {
        self.rings.lock().unwrap().push(ring);
    }
}

/// Steal-on-full ring pool.
///
/// Provides lock-free push for producers. When a ring fills, the producer
/// swaps it with a pre-fetched spare and continues at full speed.
#[cfg(feature = "std")]
pub struct StealPool<T, const N: usize> {
    empty: EmptyPool<T, N>,
    full: DrainQueue<T, N>,
}

#[cfg(feature = "std")]
impl<T, const N: usize> StealPool<T, N> {
    /// Create a new pool with `num_rings` pre-allocated empty rings.
    pub fn new(num_rings: usize) -> Arc<Self> {
        Arc::new(Self {
            empty: EmptyPool::new(num_rings),
            full: DrainQueue::new(),
        })
    }

    /// Create a producer handle.
    pub fn producer(self: &Arc<Self>) -> StealProducer<T, N> {
        let active = self.empty.steal();
        let spare = self.empty.steal();
        StealProducer {
            pool: Arc::clone(self),
            active,
            spare: AtomicPtr::new(Box::into_raw(spare)),
        }
    }

    /// Signal shutdown to drain thread.
    pub fn shutdown(&self) {
        self.full.shutdown();
    }

    /// Spawn a drain thread that processes full rings.
    pub fn spawn_drain<F>(self: &Arc<Self>, mut drain_fn: F) -> std::thread::JoinHandle<()>
    where
        T: Send + 'static,
        F: FnMut(&mut SpillRing<T, N, DropSpout>) + Send + 'static,
    {
        let pool = Arc::clone(self);
        std::thread::spawn(move || {
            while let Some(mut ring) = pool.full.pop() {
                drain_fn(&mut ring);
                ring.clear_drop();
                pool.empty.return_ring(ring);
            }
        })
    }
}

/// Producer handle with lock-free push and steal-on-full.
#[cfg(feature = "std")]
pub struct StealProducer<T, const N: usize> {
    pool: Arc<StealPool<T, N>>,
    /// Currently active ring for pushing.
    active: BoxedRing<T, N>,
    /// Pre-fetched spare ring (swapped in when active is full).
    spare: AtomicPtr<SpillRing<T, N, DropSpout>>,
}

#[cfg(feature = "std")]
impl<T, const N: usize> StealProducer<T, N> {
    /// Push an item. Lock-free on hot path.
    ///
    /// When the active ring fills, swaps with the pre-fetched spare
    /// and queues the full ring for drain.
    #[inline]
    pub fn push(&mut self, item: T) {
        if self.active.is_full() {
            self.rotate();
        }
        self.active.push(item);
    }

    /// Rotate: swap active with spare, queue full ring for drain.
    #[cold]
    fn rotate(&mut self) {
        // Swap active with spare (lock-free)
        let spare_ptr = self.spare.swap(ptr::null_mut(), Ordering::AcqRel);

        if !spare_ptr.is_null() {
            // Take ownership of spare
            let spare = unsafe { Box::from_raw(spare_ptr) };

            // Swap active and spare
            let full = core::mem::replace(&mut self.active, spare);

            // Queue full ring for drain
            self.pool.full.push(full);

            // Fetch new spare in background (this is where lock happens)
            let new_spare = self.pool.empty.steal();
            self.spare
                .store(Box::into_raw(new_spare), Ordering::Release);
        } else {
            // No spare available, fetch one now (blocking)
            let new_spare = self.pool.empty.steal();
            let full = core::mem::replace(&mut self.active, new_spare);
            self.pool.full.push(full);

            // Fetch another spare
            let another_spare = self.pool.empty.steal();
            self.spare
                .store(Box::into_raw(another_spare), Ordering::Release);
        }
    }

    /// Get current ring length.
    #[inline]
    pub fn len(&self) -> usize {
        self.active.len()
    }

    /// Check if current ring is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }
}

#[cfg(feature = "std")]
impl<T, const N: usize> Drop for StealProducer<T, N> {
    fn drop(&mut self) {
        // Queue active ring if non-empty
        if !self.active.is_empty() {
            let active = core::mem::replace(&mut self.active, Box::new(SpillRing::new()));
            self.pool.full.push(active);
        }

        // Return spare to pool
        let spare_ptr = self.spare.swap(ptr::null_mut(), Ordering::AcqRel);
        if !spare_ptr.is_null() {
            let spare = unsafe { Box::from_raw(spare_ptr) };
            self.pool.empty.return_ring(spare);
        }
    }
}

// Re-export old names for compatibility
#[cfg(feature = "std")]
pub use StealPool as RingPool;
#[cfg(feature = "std")]
pub use StealProducer as PoolProducer;

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicU64;
    use std::thread;

    #[test]
    fn basic_steal_pool() {
        let pool = StealPool::<u64, 64>::new(4);
        let count = Arc::new(AtomicU64::new(0));

        let count_clone = Arc::clone(&count);
        let drain = pool.spawn_drain(move |ring| {
            for _ in ring.drain() {
                count_clone.fetch_add(1, Ordering::Relaxed);
            }
        });

        let mut producer = pool.producer();
        for i in 0..1000 {
            producer.push(i);
        }
        drop(producer);

        pool.shutdown();
        drain.join().unwrap();

        assert_eq!(count.load(Ordering::Relaxed), 1000);
    }

    #[test]
    fn multi_producer_steal() {
        let pool = StealPool::<u64, 64>::new(16);
        let count = Arc::new(AtomicU64::new(0));

        let count_clone = Arc::clone(&count);
        let drain = pool.spawn_drain(move |ring| {
            for _ in ring.drain() {
                count_clone.fetch_add(1, Ordering::Relaxed);
            }
        });

        let handles: Vec<_> = (0..4)
            .map(|_| {
                let pool = Arc::clone(&pool);
                thread::spawn(move || {
                    let mut producer = pool.producer();
                    for i in 0..1000 {
                        producer.push(i);
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        pool.shutdown();
        drain.join().unwrap();

        assert_eq!(count.load(Ordering::Relaxed), 4000);
    }

    #[test]
    fn high_throughput_rotation() {
        // Force many rotations by using small rings
        let pool = StealPool::<u64, 16>::new(8);
        let count = Arc::new(AtomicU64::new(0));

        let count_clone = Arc::clone(&count);
        let drain = pool.spawn_drain(move |ring| {
            count_clone.fetch_add(ring.len() as u64, Ordering::Relaxed);
        });

        let mut producer = pool.producer();
        // 10000 items / 16 capacity = 625 rotations
        for i in 0..10_000 {
            producer.push(i);
        }
        drop(producer);

        pool.shutdown();
        drain.join().unwrap();

        assert_eq!(count.load(Ordering::Relaxed), 10_000);
    }
}
