//! Concurrent SPSC (Single-Producer, Single-Consumer) ring buffer.
//!
//! `SpscRing` is `Sync + Send` and designed to be shared via `Arc` between
//! a producer thread and a consumer thread. Uses Lamport-style Acquire/Release
//! ordering with no CAS operations.

use core::mem::MaybeUninit;

use crate::{
    index::{AtomicIndex, SpoutCell},
    ring::{MAX_CAPACITY, Slot},
    traits::{RingConsumer, RingInfo, RingProducer},
};
use spout::{DropSpout, Spout};

/// Target cache-line size in bytes.
const CACHE_LINE: usize = 64;

/// Padding to fill the consumer cache line (head + cached_tail + pad = CACHE_LINE).
const HEAD_PAD: usize =
    CACHE_LINE - size_of::<AtomicIndex>() - size_of::<core::cell::Cell<usize>>();

/// Padding to fill the producer cache line (tail + cached_head + evict_head + pad = CACHE_LINE).
const TAIL_PAD: usize =
    CACHE_LINE - 2 * size_of::<AtomicIndex>() - size_of::<core::cell::Cell<usize>>();

/// Concurrent SPSC ring buffer with overflow spilling to a spout.
///
/// Fields are laid out with explicit cache-line padding to prevent false
/// sharing between the producer (writes `tail`) and consumer (writes `head`).
///
/// # Thread Safety
///
/// `SpscRing` is `Sync + Send`. Share it via `Arc` for one producer thread
/// and one consumer thread. Multiple concurrent producers or consumers are
/// NOT safe.
///
/// # Eviction
///
/// When the ring is full, the producer evicts the oldest item to the spout
/// via `evict_head` (a Release store — no CAS). The consumer detects evictions
/// via a seqlock-style double-read of `evict_head`.
#[repr(C)]
pub struct SpscRing<T, const N: usize, S: Spout<T> = DropSpout> {
    // ── Consumer cache line ──────────────────────────────────────────
    head: AtomicIndex,
    cached_tail: core::cell::Cell<usize>,
    _pad_head: [u8; HEAD_PAD],

    // ── Producer cache line ──────────────────────────────────────────
    tail: AtomicIndex,
    cached_head: core::cell::Cell<usize>,
    evict_head: AtomicIndex,
    _pad_tail: [u8; TAIL_PAD],

    // ── Cold fields ──────────────────────────────────────────────────
    buffer: [Slot<T>; N],
    sink: SpoutCell<S>,
}

unsafe impl<T: Send, const N: usize, S: Spout<T> + Send> Send for SpscRing<T, N, S> {}
unsafe impl<T: Send, const N: usize, S: Spout<T> + Send> Sync for SpscRing<T, N, S> {}

// ── Constructors ─────────────────────────────────────────────────────

impl<T, const N: usize> SpscRing<T, N, DropSpout> {
    /// Create a builder for configuring a [`SpscRing`].
    ///
    /// # Example
    ///
    /// ```
    /// use spill_ring::SpscRing;
    ///
    /// let ring = SpscRing::<u64, 256>::builder()
    ///     .cold()
    ///     .build();
    /// ```
    pub fn builder() -> crate::builder::SpscRingBuilder<T, N> {
        crate::builder::SpscRingBuilder::new()
    }

    /// Create a new SPSC ring with pre-warmed cache (evicted items are dropped).
    #[must_use]
    pub fn new() -> Self {
        let ring = Self::cold();
        ring.warm();
        ring
    }

    /// Create a new SPSC ring without cache warming (evicted items are dropped).
    #[must_use]
    pub const fn cold() -> Self {
        const { assert!(N > 0, "capacity must be > 0") };
        const { assert!(N.is_power_of_two(), "capacity must be power of two") };
        const { assert!(N <= MAX_CAPACITY, "capacity exceeds maximum (2^20)") };

        Self {
            head: AtomicIndex::new(0),
            cached_tail: core::cell::Cell::new(0),
            _pad_head: [0; HEAD_PAD],
            tail: AtomicIndex::new(0),
            cached_head: core::cell::Cell::new(0),
            evict_head: AtomicIndex::new(0),
            _pad_tail: [0; TAIL_PAD],
            buffer: [const { Slot::new() }; N],
            sink: SpoutCell::new(DropSpout),
        }
    }
}

impl<T, const N: usize, S: Spout<T>> SpscRing<T, N, S> {
    /// Create a new SPSC ring with pre-warmed cache and a custom spout.
    #[must_use]
    pub fn with_sink(sink: S) -> Self {
        let ring = Self::with_sink_cold(sink);
        ring.warm();
        ring
    }

    /// Create a new SPSC ring with a custom spout, without cache warming.
    #[must_use]
    pub fn with_sink_cold(sink: S) -> Self {
        const { assert!(N > 0, "capacity must be > 0") };
        const { assert!(N.is_power_of_two(), "capacity must be power of two") };
        const { assert!(N <= MAX_CAPACITY, "capacity exceeds maximum (2^20)") };

        Self {
            head: AtomicIndex::new(0),
            cached_tail: core::cell::Cell::new(0),
            _pad_head: [0; HEAD_PAD],
            tail: AtomicIndex::new(0),
            cached_head: core::cell::Cell::new(0),
            evict_head: AtomicIndex::new(0),
            _pad_tail: [0; TAIL_PAD],
            buffer: [const { Slot::new() }; N],
            sink: SpoutCell::new(sink),
        }
    }

    /// Bring all ring slots into L1/L2 cache.
    fn warm(&self) {
        for i in 0..N {
            unsafe {
                let slot = &self.buffer[i];
                let ptr = slot.data.get() as *mut u8;
                core::ptr::write_bytes(ptr, 0, core::mem::size_of::<MaybeUninit<T>>());
            }
        }
        self.head.store(0);
        self.cached_tail.set(0);
        self.tail.store(0);
        self.cached_head.set(0);
        self.evict_head.store(0);
    }

    // ── SPSC push/pop (&self, atomic) ────────────────────────────────

    /// Push an item. If full, evicts oldest to spout.
    ///
    /// Thread-safe for single-producer use. Uses Acquire/Release ordering
    /// with no CAS. The producer exclusively owns `tail` and `evict_head`.
    #[inline]
    pub fn push(&self, item: T) {
        let tail = self.tail.load_relaxed();

        let mut head = self.cached_head.get();
        if tail.wrapping_sub(head) >= N {
            head = self.head.load();
            self.cached_head.set(head);

            if tail.wrapping_sub(head) >= N {
                let mut evict = self.evict_head.load_relaxed();
                if evict < head {
                    evict = head;
                }
                let idx = evict & (N - 1);
                let evicted = unsafe { (*self.buffer[idx].data.get()).assume_init_read() };
                unsafe { self.sink.get_mut_unchecked().send(evicted) };
                self.evict_head.store(evict.wrapping_add(1));

                // Fence: ensure evict_head publication completes before new data write.
                // On x86: compiler fence only (TSO provides hardware ordering).
                // On ARM64: DMB ISH — required to prevent store reordering.
                core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
            }
        }

        let idx = tail & (N - 1);
        unsafe { (*self.buffer[idx].data.get()).write(item) };
        self.tail.store(tail.wrapping_add(1));
    }

    /// Pop the oldest item.
    ///
    /// Thread-safe for single-consumer use. Uses a seqlock-style double-read
    /// of `evict_head` to detect when the producer evicts a slot during the
    /// consumer's read.
    #[inline]
    #[must_use]
    pub fn pop(&self) -> Option<T> {
        loop {
            let mut head = self.head.load_relaxed();

            let evict = self.evict_head.load();
            if head < evict {
                head = evict;
            }

            let mut tail = self.cached_tail.get();
            let cached_avail = tail.wrapping_sub(head);
            if cached_avail == 0 || cached_avail > N {
                tail = self.tail.load();
                self.cached_tail.set(tail);
                if head == tail {
                    if head != self.head.load_relaxed() {
                        self.head.store(head);
                    }
                    return None;
                }
            }

            let idx = head & (N - 1);
            let speculative: MaybeUninit<T> =
                unsafe { core::ptr::read(self.buffer[idx].data.get()) };

            core::sync::atomic::fence(core::sync::atomic::Ordering::Acquire);

            let evict2 = self.evict_head.load_relaxed();
            if evict2 > head {
                continue;
            }

            self.head.store(head.wrapping_add(1));
            return Some(unsafe { speculative.assume_init() });
        }
    }

    // ── Exclusive access (&mut self) ─────────────────────────────────

    /// Push with exclusive access (no atomic overhead).
    #[inline]
    pub fn push_mut(&mut self, item: T) {
        let tail = self.tail.load_mut();
        let head = self.head.load_mut();

        if tail.wrapping_sub(head) >= N {
            let evict_idx = head & (N - 1);
            let evicted = unsafe { (*self.buffer[evict_idx].data.get()).assume_init_read() };
            self.head.store_mut(head.wrapping_add(1));
            self.sink.get_mut().send(evicted);
        }

        let idx = tail & (N - 1);
        unsafe { (*self.buffer[idx].data.get()).write(item) };
        self.tail.store_mut(tail.wrapping_add(1));
    }

    /// Pop with exclusive access (no atomic overhead).
    ///
    /// Accounts for `evict_head` in case `push(&self)` was used before
    /// transitioning to exclusive access.
    #[inline]
    #[must_use]
    pub fn pop_mut(&mut self) -> Option<T> {
        let mut head = self.head.load_mut();
        let evict = self.evict_head.load_mut();
        if head < evict {
            head = evict;
        }
        let tail = self.tail.load_mut();

        if head == tail {
            self.head.store_mut(head);
            self.evict_head.store_mut(head);
            return None;
        }

        let idx = head & (N - 1);
        let item = unsafe { (*self.buffer[idx].data.get()).assume_init_read() };
        head = head.wrapping_add(1);
        self.head.store_mut(head);
        self.evict_head.store_mut(head);
        Some(item)
    }

    /// Bulk-push a slice of `Copy` items.
    #[inline]
    pub fn push_slice(&mut self, items: &[T])
    where
        T: Copy,
    {
        if items.is_empty() {
            return;
        }

        let mut tail = self.tail.load_mut();
        let mut head = self.head.load_mut();

        let keep = if items.len() > N {
            let len = tail.wrapping_sub(head);
            if len > 0 {
                let h = head;
                self.sink.get_mut().send_all((0..len).map(|i| unsafe {
                    (*self.buffer[(h.wrapping_add(i)) & (N - 1)].data.get()).assume_init_read()
                }));
            }
            let excess = items.len() - N;
            for &item in &items[..excess] {
                self.sink.get_mut().send(item);
            }
            head = head.wrapping_add(len);
            tail = head;
            self.head.store_mut(head);
            self.tail.store_mut(tail);
            self.evict_head.store_mut(head);
            &items[excess..]
        } else {
            items
        };

        let len = tail.wrapping_sub(head);
        let free = N - len;
        if keep.len() > free {
            let evict_count = keep.len() - free;
            let h = head;
            self.sink
                .get_mut()
                .send_all((0..evict_count).map(|i| unsafe {
                    (*self.buffer[(h.wrapping_add(i)) & (N - 1)].data.get()).assume_init_read()
                }));
            self.head.store_mut(head.wrapping_add(evict_count));
            self.evict_head.store_mut(head.wrapping_add(evict_count));
        }

        let tail_idx = tail & (N - 1);
        let space_to_end = N - tail_idx;
        let count = keep.len();

        unsafe {
            let dst = self.buffer[tail_idx].data.get() as *mut T;
            if count <= space_to_end {
                core::ptr::copy_nonoverlapping(keep.as_ptr(), dst, count);
            } else {
                core::ptr::copy_nonoverlapping(keep.as_ptr(), dst, space_to_end);
                core::ptr::copy_nonoverlapping(
                    keep.as_ptr().add(space_to_end),
                    self.buffer[0].data.get() as *mut T,
                    count - space_to_end,
                );
            }
        }

        self.tail.store_mut(tail.wrapping_add(count));
    }

    /// Bulk-extend from a slice. Equivalent to `push_slice`.
    #[inline]
    pub fn extend_from_slice(&mut self, items: &[T])
    where
        T: Copy,
    {
        self.push_slice(items);
    }

    /// Bulk-pop up to `buf.len()` items with exclusive access. Returns count popped.
    ///
    /// Uses `memcpy` internally — at most two copies (from head to buffer end + wrap).
    /// Accounts for `evict_head` in case `push(&self)` was used previously.
    #[inline]
    pub fn pop_slice_mut(&mut self, buf: &mut [MaybeUninit<T>]) -> usize
    where
        T: Copy,
    {
        if buf.is_empty() {
            return 0;
        }

        let mut head = self.head.load_mut();
        let evict = self.evict_head.load_mut();
        if head < evict {
            head = evict;
        }
        let tail = self.tail.load_mut();
        let avail = tail.wrapping_sub(head);
        let count = buf.len().min(avail);
        if count == 0 {
            self.head.store_mut(head);
            self.evict_head.store_mut(head);
            return 0;
        }

        let head_idx = head & (N - 1);
        let to_end = N - head_idx;

        unsafe {
            let src = self.buffer[head_idx].data.get() as *const T;
            let dst = buf.as_mut_ptr() as *mut T;
            if count <= to_end {
                core::ptr::copy_nonoverlapping(src, dst, count);
            } else {
                core::ptr::copy_nonoverlapping(src, dst, to_end);
                core::ptr::copy_nonoverlapping(
                    self.buffer[0].data.get() as *const T,
                    dst.add(to_end),
                    count - to_end,
                );
            }
        }

        head = head.wrapping_add(count);
        self.head.store_mut(head);
        self.evict_head.store_mut(head);
        count
    }

    /// Bulk-pop up to `buf.len()` items concurrently. Returns count popped.
    ///
    /// Thread-safe for single-consumer use. Uses a seqlock-style double-read
    /// of `evict_head` to detect producer evictions during the bulk read.
    /// On conflict, retries with the updated head.
    ///
    /// Amortizes the cross-core cache-line RTT over the entire batch instead
    /// of paying it per element.
    #[inline]
    pub fn pop_slice(&self, buf: &mut [MaybeUninit<T>]) -> usize
    where
        T: Copy,
    {
        if buf.is_empty() {
            return 0;
        }

        loop {
            let mut head = self.head.load_relaxed();

            let evict = self.evict_head.load();
            if head < evict {
                head = evict;
            }

            let mut tail = self.cached_tail.get();
            let cached_avail = tail.wrapping_sub(head);
            if cached_avail == 0 || cached_avail > N {
                tail = self.tail.load();
                self.cached_tail.set(tail);
                if head == tail {
                    if head != self.head.load_relaxed() {
                        self.head.store(head);
                    }
                    return 0;
                }
            }

            let avail = tail.wrapping_sub(head);
            let count = buf.len().min(avail);

            // Speculative bulk read into buf.
            let head_idx = head & (N - 1);
            let to_end = N - head_idx;

            unsafe {
                let src = self.buffer[head_idx].data.get() as *const T;
                let dst = buf.as_mut_ptr() as *mut T;
                if count <= to_end {
                    core::ptr::copy_nonoverlapping(src, dst, count);
                } else {
                    core::ptr::copy_nonoverlapping(src, dst, to_end);
                    core::ptr::copy_nonoverlapping(
                        self.buffer[0].data.get() as *const T,
                        dst.add(to_end),
                        count - to_end,
                    );
                }
            }

            core::sync::atomic::fence(core::sync::atomic::Ordering::Acquire);

            // Validate: did the producer evict any of our slots?
            let evict2 = self.evict_head.load_relaxed();
            if evict2 > head {
                // Producer overwrote slots we read — retry with new head.
                continue;
            }

            self.head.store(head.wrapping_add(count));
            return count;
        }
    }

    /// Push an item then flush all to spout.
    #[inline]
    pub fn push_and_flush(&mut self, item: T) {
        self.push_mut(item);
        self.flush();
    }

    /// Flush all items to spout. Returns count flushed.
    #[inline]
    pub fn flush(&mut self) -> usize {
        let mut head = self.head.load_mut();
        let evict = self.evict_head.load_mut();
        if head < evict {
            head = evict;
        }
        let tail = self.tail.load_mut();
        let count = tail.wrapping_sub(head);
        if count == 0 {
            return 0;
        }

        let h = head;
        self.sink.get_mut().send_all((0..count).map(|i| unsafe {
            (*self.buffer[(h.wrapping_add(i)) & (N - 1)].data.get()).assume_init_read()
        }));

        self.head.store_mut(tail);
        self.tail.store_mut(tail);
        self.evict_head.store_mut(tail);
        count
    }

    // ── Queries ──────────────────────────────────────────────────────

    /// Number of items in buffer.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        let tail = self.tail.load();
        let head = self.head.load();
        let evict = self.evict_head.load();
        let effective = if head < evict { evict } else { head };
        let len = tail.wrapping_sub(effective);
        if len > N { N } else { len }
    }

    /// True if empty.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// True if full.
    #[inline]
    #[must_use]
    pub fn is_full(&self) -> bool {
        self.len() >= N
    }

    /// Buffer capacity.
    #[inline]
    #[must_use]
    pub const fn capacity(&self) -> usize {
        N
    }

    /// Clear all items from the buffer, flushing them to the spout.
    pub fn clear(&mut self) {
        self.flush();
    }

    /// Reference to the spout.
    #[inline]
    #[must_use]
    pub fn sink(&self) -> &S {
        self.sink.get_ref()
    }

    /// Mutable reference to the spout.
    #[inline]
    pub fn sink_mut(&mut self) -> &mut S {
        self.sink.get_mut()
    }

    // ── Read accessors (&mut self required — ring is Sync) ───────────

    /// Peek at the oldest item.
    #[inline]
    #[must_use]
    pub fn peek(&mut self) -> Option<&T> {
        let head = self.head.load_mut();
        let tail = self.tail.load_mut();
        if head == tail {
            return None;
        }
        Some(unsafe { (*self.buffer[head & (N - 1)].data.get()).assume_init_ref() })
    }

    /// Peek at the newest item.
    #[inline]
    #[must_use]
    pub fn peek_back(&mut self) -> Option<&T> {
        let head = self.head.load_mut();
        let tail = self.tail.load_mut();
        if head == tail {
            return None;
        }
        let idx = tail.wrapping_sub(1) & (N - 1);
        Some(unsafe { (*self.buffer[idx].data.get()).assume_init_ref() })
    }

    /// Get item by index (0 = oldest).
    #[inline]
    #[must_use]
    pub fn get(&mut self, index: usize) -> Option<&T> {
        let head = self.head.load_mut();
        let tail = self.tail.load_mut();
        let len = tail.wrapping_sub(head);
        if index >= len {
            return None;
        }
        let idx = head.wrapping_add(index) & (N - 1);
        Some(unsafe { (*self.buffer[idx].data.get()).assume_init_ref() })
    }

    /// Iterate oldest to newest (requires `&mut self`).
    #[inline]
    pub fn iter(&mut self) -> SpscRingIter<'_, T, N, S> {
        let head = self.head.load_mut();
        let tail = self.tail.load_mut();
        SpscRingIter {
            ring: self,
            pos: 0,
            len: tail.wrapping_sub(head),
            head,
        }
    }

    /// Iterate mutably, oldest to newest.
    #[inline]
    pub fn iter_mut(&mut self) -> SpscRingIterMut<'_, T, N, S> {
        let head = self.head.load_mut();
        let tail = self.tail.load_mut();
        SpscRingIterMut {
            ring: self,
            pos: 0,
            len: tail.wrapping_sub(head),
            head,
        }
    }

    /// Drain all items from the ring, returning an iterator.
    #[inline]
    pub fn drain(&mut self) -> SpscDrain<'_, T, N, S> {
        SpscDrain { ring: self }
    }
}

// ── Iterators ────────────────────────────────────────────────────────

/// Immutable iterator over a SpscRing.
pub struct SpscRingIter<'a, T, const N: usize, S: Spout<T>> {
    ring: &'a SpscRing<T, N, S>,
    pos: usize,
    len: usize,
    head: usize,
}

impl<'a, T, const N: usize, S: Spout<T>> Iterator for SpscRingIter<'a, T, N, S> {
    type Item = &'a T;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.len {
            return None;
        }
        let idx = self.head.wrapping_add(self.pos) & (N - 1);
        self.pos += 1;
        Some(unsafe { (*self.ring.buffer[idx].data.get()).assume_init_ref() })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.len - self.pos;
        (remaining, Some(remaining))
    }
}

impl<T, const N: usize, S: Spout<T>> ExactSizeIterator for SpscRingIter<'_, T, N, S> {}

/// Mutable iterator over a SpscRing.
pub struct SpscRingIterMut<'a, T, const N: usize, S: Spout<T>> {
    ring: &'a SpscRing<T, N, S>,
    pos: usize,
    len: usize,
    head: usize,
}

impl<'a, T, const N: usize, S: Spout<T>> Iterator for SpscRingIterMut<'a, T, N, S> {
    type Item = &'a mut T;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.len {
            return None;
        }
        let idx = self.head.wrapping_add(self.pos) & (N - 1);
        self.pos += 1;
        Some(unsafe { &mut *(*self.ring.buffer[idx].data.get()).as_mut_ptr() })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.len - self.pos;
        (remaining, Some(remaining))
    }
}

impl<T, const N: usize, S: Spout<T>> ExactSizeIterator for SpscRingIterMut<'_, T, N, S> {}

/// Draining iterator over a SpscRing.
pub struct SpscDrain<'a, T, const N: usize, S: Spout<T>> {
    ring: &'a mut SpscRing<T, N, S>,
}

impl<T, const N: usize, S: Spout<T>> Iterator for SpscDrain<'_, T, N, S> {
    type Item = T;

    #[inline]
    fn next(&mut self) -> Option<T> {
        self.ring.pop_mut()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.ring.len();
        (len, Some(len))
    }
}

impl<T, const N: usize, S: Spout<T>> ExactSizeIterator for SpscDrain<'_, T, N, S> {}

// ── Trait impls ──────────────────────────────────────────────────────

impl<T, const N: usize, S: Spout<T>> core::iter::Extend<T> for SpscRing<T, N, S> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        for item in iter {
            self.push_mut(item);
        }
    }
}

impl<T, const N: usize> Default for SpscRing<T, N, DropSpout> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const N: usize, S: Spout<T>> Spout<T> for SpscRing<T, N, S> {
    #[inline]
    fn send(&mut self, item: T) {
        self.push_mut(item);
    }

    #[inline]
    fn flush(&mut self) {
        SpscRing::flush(self);
    }
}

impl<T, const N: usize, S: Spout<T>> Drop for SpscRing<T, N, S> {
    fn drop(&mut self) {
        self.flush();
        self.sink.get_mut().flush();
    }
}

impl<T, const N: usize, S: Spout<T>> RingInfo for SpscRing<T, N, S> {
    #[inline]
    fn len(&self) -> usize {
        SpscRing::len(self)
    }

    #[inline]
    fn capacity(&self) -> usize {
        N
    }
}

impl<T, const N: usize, S: Spout<T>> RingProducer<T> for SpscRing<T, N, S> {
    #[inline]
    fn try_push(&mut self, item: T) -> Result<(), T> {
        let tail = self.tail.load_mut();
        let head = self.head.load_mut();

        if tail.wrapping_sub(head) >= N {
            return Err(item);
        }

        unsafe {
            let slot = &self.buffer[tail & (N - 1)];
            (*slot.data.get()).write(item);
        }
        self.tail.store_mut(tail.wrapping_add(1));
        Ok(())
    }
}

impl<T, const N: usize, S: Spout<T>> RingConsumer<T> for SpscRing<T, N, S> {
    #[inline]
    fn try_pop(&mut self) -> Option<T> {
        self.pop_mut()
    }

    #[inline]
    fn peek(&mut self) -> Option<&T> {
        SpscRing::peek(self)
    }
}

// ── SPSC Producer/Consumer handles ───────────────────────────────────

#[cfg(feature = "std")]
mod handles {
    extern crate alloc;
    use alloc::sync::Arc;
    use core::mem::MaybeUninit;
    use spout::Spout;

    use super::SpscRing;

    impl<T: Send, const N: usize, S: Spout<T> + Send> SpscRing<T, N, S> {
        /// Split into typed producer and consumer handles for safe concurrent use.
        ///
        /// The producer can only push; the consumer can only pop. Neither handle
        /// is `Clone`, enforcing the single-producer/single-consumer contract at
        /// compile time.
        ///
        /// Both handles hold an `Arc` to the underlying ring.
        ///
        /// # Example
        ///
        /// ```
        /// use spill_ring::SpscRing;
        /// use std::thread;
        ///
        /// let (producer, consumer) = SpscRing::<u64, 256>::new().split();
        ///
        /// let t = thread::spawn(move || {
        ///     for i in 0..100 {
        ///         producer.push(i);
        ///     }
        /// });
        ///
        /// // Consumer reads from another context
        /// t.join().unwrap();
        /// while let Some(val) = consumer.pop() {
        ///     // process val
        /// }
        /// ```
        pub fn split(self) -> (SpscProducer<T, N, S>, SpscConsumer<T, N, S>) {
            let arc = Arc::new(self);
            (
                SpscProducer {
                    ring: Arc::clone(&arc),
                },
                SpscConsumer { ring: arc },
            )
        }
    }

    /// Producer handle for a [`SpscRing`]. Can only push.
    ///
    /// Created by [`SpscRing::split()`]. Not `Clone` — only one producer
    /// is permitted per ring.
    pub struct SpscProducer<T, const N: usize, S: Spout<T>> {
        ring: Arc<SpscRing<T, N, S>>,
    }

    // Safety: SpscProducer is Send if T and S are Send.
    // It is NOT Sync — the producer handle should only be used from one thread.
    unsafe impl<T: Send, const N: usize, S: Spout<T> + Send> Send for SpscProducer<T, N, S> {}

    impl<T: Send, const N: usize, S: Spout<T> + Send> SpscProducer<T, N, S> {
        /// Push an item. If the ring is full, evicts the oldest item to the spout.
        #[inline]
        pub fn push(&self, item: T) {
            self.ring.push(item);
        }

        /// Number of items currently in the ring.
        #[inline]
        pub fn len(&self) -> usize {
            self.ring.len()
        }

        /// True if the ring is empty.
        #[inline]
        pub fn is_empty(&self) -> bool {
            self.ring.is_empty()
        }

        /// True if the ring is full.
        #[inline]
        pub fn is_full(&self) -> bool {
            self.ring.is_full()
        }

        /// Buffer capacity.
        #[inline]
        pub fn capacity(&self) -> usize {
            self.ring.capacity()
        }
    }

    /// Consumer handle for a [`SpscRing`]. Can only pop.
    ///
    /// Created by [`SpscRing::split()`]. Not `Clone` — only one consumer
    /// is permitted per ring.
    pub struct SpscConsumer<T, const N: usize, S: Spout<T>> {
        ring: Arc<SpscRing<T, N, S>>,
    }

    // Safety: SpscConsumer is Send if T and S are Send.
    // It is NOT Sync — the consumer handle should only be used from one thread.
    unsafe impl<T: Send, const N: usize, S: Spout<T> + Send> Send for SpscConsumer<T, N, S> {}

    impl<T: Send, const N: usize, S: Spout<T> + Send> SpscConsumer<T, N, S> {
        /// Pop the oldest item.
        #[inline]
        pub fn pop(&self) -> Option<T> {
            self.ring.pop()
        }

        /// Bulk-pop up to `buf.len()` items concurrently.
        #[inline]
        pub fn pop_slice(&self, buf: &mut [MaybeUninit<T>]) -> usize
        where
            T: Copy,
        {
            self.ring.pop_slice(buf)
        }

        /// Number of items currently in the ring.
        #[inline]
        pub fn len(&self) -> usize {
            self.ring.len()
        }

        /// True if the ring is empty.
        #[inline]
        pub fn is_empty(&self) -> bool {
            self.ring.is_empty()
        }

        /// True if the ring is full.
        #[inline]
        pub fn is_full(&self) -> bool {
            self.ring.is_full()
        }

        /// Buffer capacity.
        #[inline]
        pub fn capacity(&self) -> usize {
            self.ring.capacity()
        }
    }
}

#[cfg(feature = "std")]
pub use handles::{SpscConsumer, SpscProducer};

// ── Layout test ──────────────────────────────────────────────────────

#[cfg(test)]
mod layout_tests {
    use super::*;
    use core::mem;

    type Ring = SpscRing<u64, 8>;

    #[test]
    fn cache_line_layout() {
        let head_offset = mem::offset_of!(Ring, head);
        let cached_tail_offset = mem::offset_of!(Ring, cached_tail);
        let tail_offset = mem::offset_of!(Ring, tail);
        let cached_head_offset = mem::offset_of!(Ring, cached_head);
        let evict_head_offset = mem::offset_of!(Ring, evict_head);
        let buffer_offset = mem::offset_of!(Ring, buffer);

        assert_eq!(head_offset, 0, "head should be at offset 0");
        assert_eq!(
            cached_tail_offset,
            size_of::<AtomicIndex>(),
            "cached_tail should follow head"
        );

        assert_eq!(
            tail_offset, CACHE_LINE,
            "tail should be at start of second cache line"
        );
        assert_eq!(
            cached_head_offset,
            CACHE_LINE + size_of::<AtomicIndex>(),
            "cached_head should follow tail"
        );
        assert_eq!(
            evict_head_offset,
            CACHE_LINE + size_of::<AtomicIndex>() + size_of::<core::cell::Cell<usize>>(),
            "evict_head should follow cached_head"
        );

        assert_eq!(
            buffer_offset,
            2 * CACHE_LINE,
            "buffer should start at third cache line"
        );

        assert_ne!(
            head_offset / CACHE_LINE,
            tail_offset / CACHE_LINE,
            "head and tail must be on different cache lines"
        );

        assert_eq!(
            cached_tail_offset / CACHE_LINE,
            head_offset / CACHE_LINE,
            "cached_tail must share cache line with head"
        );
        assert_eq!(
            cached_head_offset / CACHE_LINE,
            tail_offset / CACHE_LINE,
            "cached_head must share cache line with tail"
        );
        assert_eq!(
            evict_head_offset / CACHE_LINE,
            tail_offset / CACHE_LINE,
            "evict_head must share cache line with tail"
        );
    }
}
