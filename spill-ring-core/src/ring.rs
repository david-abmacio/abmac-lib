//! Ring buffer with overflow spilling to a sink.

#[cfg(feature = "atomics")]
use core::sync::atomic::{AtomicUsize, Ordering};
use core::{cell::UnsafeCell, mem::MaybeUninit};

use crate::{
    index::{Index, SinkCell},
    iter::{SpillRingIter, SpillRingIterMut},
    sink::{DropSink, Sink},
    traits::{RingConsumer, RingInfo, RingProducer},
};

/// Slot wrapper with seqlock for safe concurrent access.
#[cfg(feature = "atomics")]
pub(crate) struct Slot<T> {
    /// Sequence number: odd = operation in progress, even = slot is free.
    /// Used to coordinate access between producer and consumer on the same slot.
    seq: AtomicUsize,
    pub(crate) data: UnsafeCell<MaybeUninit<T>>,
}

#[cfg(feature = "atomics")]
impl<T> Slot<T> {
    const fn new() -> Self {
        Self {
            seq: AtomicUsize::new(0),
            data: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }
}

/// Slot wrapper without seqlock for single-threaded use.
#[cfg(not(feature = "atomics"))]
pub(crate) struct Slot<T> {
    pub(crate) data: UnsafeCell<MaybeUninit<T>>,
}

#[cfg(not(feature = "atomics"))]
impl<T> Slot<T> {
    const fn new() -> Self {
        Self {
            data: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }
}

/// Ring buffer that spills evicted items to a sink.
pub struct SpillRing<T, const N: usize, S: Sink<T> = DropSink> {
    pub(crate) buffer: [Slot<T>; N],
    pub(crate) head: Index,
    pub(crate) tail: Index,
    sink: SinkCell<S>,
}

unsafe impl<T: Send, const N: usize, S: Sink<T> + Send> Send for SpillRing<T, N, S> {}

#[cfg(feature = "atomics")]
unsafe impl<T: Send, const N: usize, S: Sink<T> + Send> Sync for SpillRing<T, N, S> {}

/// Maximum supported capacity (2^20 = ~1 million slots).
/// Prevents accidental huge allocations from typos like `SpillRing<T, 1000000000>`.
const MAX_CAPACITY: usize = 1 << 20;

impl<T, const N: usize> SpillRing<T, N, DropSink> {
    /// Create a new ring buffer (evicted items are dropped).
    #[must_use]
    pub const fn new() -> Self {
        const { assert!(N > 0, "capacity must be > 0") };
        const { assert!(N.is_power_of_two(), "capacity must be power of two") };
        const { assert!(N <= MAX_CAPACITY, "capacity exceeds maximum (2^20)") };

        Self {
            buffer: [const { Slot::new() }; N],
            head: Index::new(0),
            tail: Index::new(0),
            sink: SinkCell::new(DropSink),
        }
    }
}

impl<T, const N: usize, S: Sink<T>> SpillRing<T, N, S> {
    /// Create a new ring buffer with a custom sink.
    #[must_use]
    pub fn with_sink(sink: S) -> Self {
        const { assert!(N > 0, "capacity must be > 0") };
        const { assert!(N.is_power_of_two(), "capacity must be power of two") };
        const { assert!(N <= MAX_CAPACITY, "capacity exceeds maximum (2^20)") };

        Self {
            buffer: [const { Slot::new() }; N],
            head: Index::new(0),
            tail: Index::new(0),
            sink: SinkCell::new(sink),
        }
    }

    /// Push an item. If full, evicts oldest to sink.
    ///
    /// Thread-safe for single-producer, single-consumer (SPSC) use.
    /// Multiple concurrent pushes or multiple concurrent pops are NOT safe.
    ///
    /// When the buffer is full, the oldest item is evicted to the sink.
    #[inline]
    #[cfg(feature = "atomics")]
    pub fn push(&self, item: T) {
        // Load current tail (only producer modifies tail, so relaxed is fine)
        let tail = self.tail.load_relaxed();

        // Ensure there's room: evict until tail - head < N
        loop {
            let head = self.head.load();
            if tail.wrapping_sub(head) < N {
                break; // There's room
            }

            // Buffer is full - need to evict
            let evict_idx = head % N;
            let slot = &self.buffer[evict_idx];

            // CRITICAL: Claim the seqlock BEFORE head CAS
            let seq = slot.seq.load(Ordering::Acquire);
            if seq & 1 != 0 {
                // Odd = someone else has it (consumer reading), spin
                core::hint::spin_loop();
                continue;
            }

            // Try to claim the seqlock
            if slot
                .seq
                .compare_exchange(
                    seq,
                    seq.wrapping_add(1),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_err()
            {
                // Lost the race, retry
                continue;
            }

            // We have the seqlock - now try to advance head
            match self.head.compare_exchange(head, head.wrapping_add(1)) {
                Ok(_) => {
                    // Success! Read and evict the item
                    let evicted = unsafe { (*slot.data.get()).assume_init_read() };

                    // Release the slot (set seq to even)
                    slot.seq.store(seq.wrapping_add(2), Ordering::Release);

                    // Send to sink
                    unsafe { self.sink.get_mut_unchecked().send(evicted) };
                    break; // Made room
                }
                Err(_) => {
                    // Head CAS failed (consumer got it first)
                    // Release the seqlock and retry
                    slot.seq.store(seq.wrapping_add(2), Ordering::Release);
                    continue;
                }
            }
        }

        // Calculate write index
        let idx = tail % N;
        let slot = &self.buffer[idx];

        // Claim the slot for writing by setting seq to odd
        let seq = loop {
            let seq = slot.seq.load(Ordering::Acquire);
            if seq & 1 == 0 {
                // Even = slot is free, try to claim for write
                if slot
                    .seq
                    .compare_exchange(
                        seq,
                        seq.wrapping_add(1),
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
                    .is_ok()
                {
                    break seq; // Claimed, return the seq value
                }
            }
            core::hint::spin_loop();
        };

        // Write the data (we have exclusive access)
        unsafe { (*slot.data.get()).write(item) };

        // Release the slot (set seq to even)
        slot.seq.store(seq.wrapping_add(2), Ordering::Release);

        // Publish the write by incrementing tail
        self.tail.store(tail.wrapping_add(1));
    }

    /// Push an item. If full, evicts oldest to sink.
    /// (Non-atomic version for single-threaded use)
    #[inline]
    #[cfg(not(feature = "atomics"))]
    pub fn push(&self, item: T) {
        let tail = self.tail.load_relaxed();
        let idx = tail % N;

        // Evict if full
        let head = self.head.load();
        if tail.wrapping_sub(head) >= N {
            let evict_idx = head % N;
            let evicted = unsafe { (*self.buffer[evict_idx].data.get()).assume_init_read() };
            self.head.store(head.wrapping_add(1));
            unsafe { self.sink.get_mut_unchecked().send(evicted) };
        }

        // Write item - use black_box to prevent the compiler from optimizing away
        // the write when called across library boundaries
        unsafe { (*self.buffer[idx].data.get()).write(core::hint::black_box(item)) };
        self.tail.store(tail.wrapping_add(1));
    }

    /// Push an item then flush all to sink.
    #[inline]
    pub fn push_and_flush(&mut self, item: T) {
        self.push(item);
        self.flush();
    }

    /// Flush all items to sink. Returns count flushed.
    #[inline]
    pub fn flush(&mut self) -> usize {
        unsafe { self.flush_unchecked() }
    }

    /// # Safety
    /// Consumer context only.
    pub unsafe fn flush_unchecked(&self) -> usize {
        let mut count = 0;
        while let Some(item) = self.pop() {
            unsafe { self.sink.get_mut_unchecked().send(item) };
            count += 1;
        }
        count
    }

    /// Pop the oldest item.
    ///
    /// Thread-safe for single-producer, single-consumer (SPSC) use.
    /// Multiple concurrent pushes or multiple concurrent pops are NOT safe.
    #[inline]
    #[must_use]
    #[cfg(feature = "atomics")]
    pub fn pop(&self) -> Option<T> {
        loop {
            let head = self.head.load();
            let tail = self.tail.load();

            // Check if empty
            if head == tail {
                return None;
            }

            let idx = head % N;
            let slot = &self.buffer[idx];

            // CRITICAL: Claim the slot BEFORE the head CAS
            // This prevents producer from overwriting our data
            let seq = slot.seq.load(Ordering::Acquire);
            if seq & 1 != 0 {
                // Odd = someone else has it, spin and retry
                core::hint::spin_loop();
                continue;
            }

            // Try to claim the seqlock
            if slot
                .seq
                .compare_exchange(
                    seq,
                    seq.wrapping_add(1),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_err()
            {
                // Lost the race, retry
                continue;
            }

            // We have the seqlock - now try to advance head
            match self.head.compare_exchange(head, head.wrapping_add(1)) {
                Ok(_) => {
                    // Success! Read the data
                    let item = unsafe { (*slot.data.get()).assume_init_read() };

                    // Release the slot by setting seq to even
                    slot.seq.store(seq.wrapping_add(2), Ordering::Release);

                    return Some(item);
                }
                Err(_) => {
                    // Head CAS failed (producer evicted this slot)
                    // Release the seqlock and retry
                    slot.seq.store(seq.wrapping_add(2), Ordering::Release);
                    continue;
                }
            }
        }
    }

    /// Pop the oldest item. (Non-atomic version)
    #[inline]
    #[must_use]
    #[cfg(not(feature = "atomics"))]
    pub fn pop(&self) -> Option<T> {
        let head = self.head.load();
        let tail = self.tail.load();

        if head == tail {
            return None;
        }

        let idx = head % N;
        let item = unsafe { (*self.buffer[idx].data.get()).assume_init_read() };
        self.head.store(head.wrapping_add(1));
        Some(item)
    }

    /// Peek at the oldest item.
    ///
    /// Note: In concurrent SPSC use, the peeked reference may become invalid
    /// if producer evicts this item. Use with caution or prefer `pop()`.
    #[inline]
    #[must_use]
    pub fn peek(&self) -> Option<&T> {
        let head = self.head.load_relaxed();
        let tail = self.tail.load();

        if head == tail {
            return None;
        }

        Some(unsafe {
            let slot = &self.buffer[head % N];
            (*slot.data.get()).assume_init_ref()
        })
    }

    /// Peek at the newest item.
    ///
    /// Note: In concurrent SPSC use, the peeked reference may become invalid
    /// if producer overwrites this slot. Use with caution.
    #[inline]
    #[must_use]
    pub fn peek_back(&self) -> Option<&T> {
        let head = self.head.load_relaxed();
        let tail = self.tail.load();

        if head == tail {
            return None;
        }

        let idx = tail.wrapping_sub(1) % N;
        Some(unsafe {
            let slot = &self.buffer[idx];
            (*slot.data.get()).assume_init_ref()
        })
    }

    /// Number of items in buffer.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.tail.load().wrapping_sub(self.head.load())
    }

    /// True if empty.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.head.load() == self.tail.load()
    }

    /// True if full.
    #[inline]
    #[must_use]
    pub fn is_full(&self) -> bool {
        self.tail.load().wrapping_sub(self.head.load()) >= N
    }

    /// Buffer capacity.
    #[inline]
    #[must_use]
    pub const fn capacity(&self) -> usize {
        N
    }

    /// Clear buffer, flushing to sink.
    pub fn clear(&mut self) {
        self.flush();
    }

    /// Clear buffer, dropping items (bypasses sink).
    pub fn clear_drop(&self) {
        while self.pop().is_some() {}
    }

    /// Reference to the sink.
    #[inline]
    #[must_use]
    pub fn sink(&self) -> &S {
        self.sink.get_ref()
    }

    /// # Safety
    /// Consumer context only.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn sink_mut_unchecked(&self) -> &mut S {
        unsafe { self.sink.get_mut_unchecked() }
    }

    /// Get item by index (0 = oldest).
    ///
    /// Note: In concurrent SPSC use, the reference may become invalid.
    #[inline]
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&T> {
        let head = self.head.load_relaxed();
        let tail = self.tail.load();
        let len = tail.wrapping_sub(head);

        if index >= len {
            return None;
        }

        let idx = head.wrapping_add(index) % N;
        Some(unsafe {
            let slot = &self.buffer[idx];
            (*slot.data.get()).assume_init_ref()
        })
    }

    /// Iterate oldest to newest.
    #[inline]
    pub fn iter(&self) -> SpillRingIter<'_, T, N, S> {
        SpillRingIter::new(self)
    }

    /// Iterate mutably, oldest to newest.
    #[inline]
    pub fn iter_mut(&mut self) -> SpillRingIterMut<'_, T, N, S> {
        SpillRingIterMut::new(self)
    }

    /// Drain all items from the ring, returning an iterator.
    /// Items are removed oldest to newest.
    #[inline]
    pub fn drain(&mut self) -> Drain<'_, T, N, S> {
        Drain { ring: self }
    }
}

/// Draining iterator over a SpillRing.
pub struct Drain<'a, T, const N: usize, S: Sink<T>> {
    ring: &'a mut SpillRing<T, N, S>,
}

impl<T, const N: usize, S: Sink<T>> Iterator for Drain<'_, T, N, S> {
    type Item = T;

    #[inline]
    fn next(&mut self) -> Option<T> {
        self.ring.pop()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.ring.len();
        (len, Some(len))
    }
}

impl<T, const N: usize, S: Sink<T>> ExactSizeIterator for Drain<'_, T, N, S> {}

impl<T, const N: usize, S: Sink<T>> core::iter::Extend<T> for SpillRing<T, N, S> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        for item in iter {
            self.push(item);
        }
    }
}

impl<T, const N: usize> Default for SpillRing<T, N, DropSink> {
    fn default() -> Self {
        Self::new()
    }
}

/// SpillRing can act as a Sink, enabling ring chaining (ring1 -> ring2).
///
/// When used as a sink, items are pushed to the ring. If the ring overflows,
/// items spill to the ring's own sink, creating a cascade.
impl<T, const N: usize, S: Sink<T>> Sink<T> for SpillRing<T, N, S> {
    #[inline]
    fn send(&mut self, item: T) {
        self.push(item);
    }

    #[inline]
    fn flush(&mut self) {
        // Flush remaining items in this ring to its sink
        SpillRing::flush(self);
    }
}

impl<T, const N: usize, S: Sink<T>> Drop for SpillRing<T, N, S> {
    fn drop(&mut self) {
        self.flush();
        self.sink.get_mut().flush();
    }
}

impl<T, const N: usize, S: Sink<T>> RingInfo for SpillRing<T, N, S> {
    #[inline]
    fn len(&self) -> usize {
        SpillRing::len(self)
    }

    #[inline]
    fn capacity(&self) -> usize {
        N
    }
}

impl<T, const N: usize, S: Sink<T>> RingProducer<T> for SpillRing<T, N, S> {
    #[inline]
    fn try_push(&mut self, item: T) -> Result<(), T> {
        let tail = self.tail.load_relaxed();
        let head = self.head.load();

        if tail.wrapping_sub(head) >= N {
            return Err(item);
        }

        unsafe {
            let slot = &self.buffer[tail % N];
            (*slot.data.get()).write(item);
        }
        self.tail.store(tail.wrapping_add(1));

        Ok(())
    }
}

impl<T, const N: usize, S: Sink<T>> RingConsumer<T> for SpillRing<T, N, S> {
    #[inline]
    fn try_pop(&mut self) -> Option<T> {
        self.pop()
    }

    #[inline]
    fn peek(&self) -> Option<&T> {
        SpillRing::peek(self)
    }
}
