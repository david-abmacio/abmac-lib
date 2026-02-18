//! Ring buffer with overflow spilling to a spout.

use core::{cell::UnsafeCell, mem::MaybeUninit};

use crate::{
    index::{CellIndex, SpoutCell},
    iter::SpillRingIterMut,
    traits::{RingConsumer, RingInfo, RingProducer},
};
use spout::{DropSpout, Spout};

/// Slot wrapper holding one item in the ring buffer.
///
/// `#[repr(transparent)]` guarantees `[Slot<T>; N]` has the same layout
/// as `[T; N]`, enabling bulk `memcpy` via `push_slice`.
#[repr(transparent)]
pub(crate) struct Slot<T> {
    pub(crate) data: UnsafeCell<MaybeUninit<T>>,
}

impl<T> Slot<T> {
    pub(crate) const fn new() -> Self {
        Self {
            data: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }
}

/// Compile-time proof that `Slot<T>` has identical layout to `T`.
/// Required for `push_slice`/`pop_slice` bulk `copy_nonoverlapping`.
const _: () = {
    // #[repr(transparent)] over UnsafeCell<MaybeUninit<T>> guarantees this,
    // but an explicit assertion catches accidental changes to Slot's definition.
    assert!(
        core::mem::size_of::<Slot<u8>>() == core::mem::size_of::<u8>()
            && core::mem::align_of::<Slot<u8>>() == core::mem::align_of::<u8>()
    );
    assert!(
        core::mem::size_of::<Slot<u64>>() == core::mem::size_of::<u64>()
            && core::mem::align_of::<Slot<u64>>() == core::mem::align_of::<u64>()
    );
    assert!(
        core::mem::size_of::<Slot<u128>>() == core::mem::size_of::<u128>()
            && core::mem::align_of::<Slot<u128>>() == core::mem::align_of::<u128>()
    );
};

/// Maximum supported capacity (2^20 = ~1 million slots).
/// Prevents accidental huge allocations from typos like `SpillRing<T, 1000000000>`.
pub(crate) const MAX_CAPACITY: usize = 1 << 20;

/// Ring buffer that spills evicted items to a spout.
///
/// Single-threaded ring using `Cell`-based indices. For multi-threaded use,
/// see [`MpscRing`](crate::MpscRing).
#[repr(C)]
pub struct SpillRing<T, const N: usize, S: Spout<T, Error = core::convert::Infallible> = DropSpout>
{
    pub(crate) head: CellIndex,
    pub(crate) tail: CellIndex,
    pub(crate) buffer: [Slot<T>; N],
    sink: SpoutCell<S>,
}

// SAFETY: SpillRing uses interior mutability (Cell/UnsafeCell) for single-threaded
// access only (!Sync). Sending to another thread is safe when T and S are Send.
unsafe impl<T: Send, const N: usize, S: Spout<T, Error = core::convert::Infallible> + Send> Send
    for SpillRing<T, N, S>
{
}

impl<T, const N: usize> SpillRing<T, N, DropSpout> {
    /// Create a builder for configuring a [`SpillRing`].
    ///
    /// # Example
    ///
    /// ```
    /// use spill_ring::SpillRing;
    ///
    /// let ring = SpillRing::<u64, 256>::builder()
    ///     .cold()
    ///     .build();
    /// ```
    pub fn builder() -> crate::builder::SpillRingBuilder<T, N> {
        crate::builder::SpillRingBuilder::new()
    }

    /// Create a new ring buffer with pre-warmed cache (evicted items are dropped).
    #[must_use]
    pub fn new() -> Self {
        let mut ring = Self::cold();
        ring.warm();
        ring
    }

    /// Create a new ring buffer without cache warming (evicted items are dropped).
    ///
    /// Use this only in constrained environments (embedded, const contexts)
    /// where the warming overhead is unacceptable. Prefer [`new()`](Self::new)
    /// for all other cases.
    #[must_use]
    pub const fn cold() -> Self {
        const { assert!(N > 0, "capacity must be > 0") };
        const { assert!(N.is_power_of_two(), "capacity must be power of two") };
        const { assert!(N <= MAX_CAPACITY, "capacity exceeds maximum (2^20)") };

        Self {
            head: CellIndex::new(0),
            tail: CellIndex::new(0),
            buffer: [const { Slot::new() }; N],
            sink: SpoutCell::new(DropSpout),
        }
    }
}

impl<T, const N: usize, S: Spout<T, Error = core::convert::Infallible>> SpillRing<T, N, S> {
    /// Create a new ring buffer with pre-warmed cache and a custom spout.
    #[must_use]
    pub fn with_sink(sink: S) -> Self {
        let mut ring = Self::with_sink_cold(sink);
        ring.warm();
        ring
    }

    /// Create a new ring buffer with a custom spout, without cache warming.
    #[must_use]
    pub fn with_sink_cold(sink: S) -> Self {
        const { assert!(N > 0, "capacity must be > 0") };
        const { assert!(N.is_power_of_two(), "capacity must be power of two") };
        const { assert!(N <= MAX_CAPACITY, "capacity exceeds maximum (2^20)") };

        Self {
            head: CellIndex::new(0),
            tail: CellIndex::new(0),
            buffer: [const { Slot::new() }; N],
            sink: SpoutCell::new(sink),
        }
    }

    /// Bring all ring slots into L1/L2 cache.
    fn warm(&mut self) {
        for i in 0..N {
            // SAFETY: `&mut self` guarantees exclusive access. Writing zeros to
            // MaybeUninit memory is always valid — this is cache prefetching, not
            // initialization.
            unsafe {
                let slot = &mut self.buffer[i];
                let ptr = slot.data.get_mut() as *mut MaybeUninit<T> as *mut u8;
                core::ptr::write_bytes(ptr, 0, core::mem::size_of::<MaybeUninit<T>>());
            }
        }
        self.head.store_mut(0);
        self.tail.store_mut(0);
    }

    /// Push an item. If full, evicts oldest to spout.
    ///
    /// Uses interior mutability (`Cell`). Not thread-safe.
    #[inline]
    pub fn push(&self, item: T) {
        let tail = self.tail.load();
        let head = self.head.load();

        if tail.wrapping_sub(head) >= N {
            let evict_idx = head & (N - 1);
            // SAFETY: Slot at head is initialized (pushed earlier). assume_init_read
            // moves the value out; head is advanced immediately after.
            let evicted = unsafe { (*self.buffer[evict_idx].data.get()).assume_init_read() };
            self.head.store(head.wrapping_add(1));
            // SAFETY: SpillRing is !Sync, so &self proves single-context access.
            // No &mut S alias can exist because that requires &mut self.
            let _ = unsafe { self.sink.get_mut_unchecked().send(evicted) };
        }

        let idx = tail & (N - 1);
        // SAFETY: Slot at tail is uninitialized (or was previously read out).
        // write() does not drop the previous value.
        unsafe { (*self.buffer[idx].data.get()).write(item) };
        self.tail.store(tail.wrapping_add(1));
    }

    /// Push an item with exclusive access (no `Cell` overhead).
    #[inline]
    pub fn push_mut(&mut self, item: T) {
        let tail = self.tail.load_mut();
        let head = self.head.load_mut();

        if tail.wrapping_sub(head) >= N {
            let evict_idx = head & (N - 1);
            // SAFETY: Slot at head is initialized. assume_init_read moves value out.
            let evicted = unsafe { (*self.buffer[evict_idx].data.get()).assume_init_read() };
            self.head.store_mut(head.wrapping_add(1));
            let _ = self.sink.get_mut().send(evicted);
        }

        let idx = tail & (N - 1);
        // SAFETY: Slot at tail is uninitialized. write() does not drop previous value.
        unsafe { (*self.buffer[idx].data.get()).write(item) };
        self.tail.store_mut(tail.wrapping_add(1));
    }

    /// Pop the oldest item with exclusive access (no `Cell` overhead).
    #[inline]
    #[must_use]
    pub fn pop_mut(&mut self) -> Option<T> {
        let head = self.head.load_mut();
        let tail = self.tail.load_mut();

        if head == tail {
            return None;
        }

        let idx = head & (N - 1);
        // SAFETY: Slot at head is initialized (head != tail). assume_init_read moves value out.
        let item = unsafe { (*self.buffer[idx].data.get()).assume_init_read() };
        self.head.store_mut(head.wrapping_add(1));
        Some(item)
    }

    /// Bulk-push a slice of `Copy` items.
    ///
    /// Uses `memcpy` internally — at most two copies (to buffer end + wrap).
    /// Items that overflow the ring are evicted to the spout. If the slice
    /// is larger than the ring capacity, excess items go directly to the spout
    /// without touching the buffer.
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

        // If slice exceeds capacity, evict ring + send excess directly to spout.
        let keep = if items.len() > N {
            let len = tail.wrapping_sub(head);
            if len > 0 {
                let h = head;
                // SAFETY: Slots [head..tail) are initialized. Each index is consumed once.
                let _ = self.sink.get_mut().send_all((0..len).map(|i| unsafe {
                    (*self.buffer[(h.wrapping_add(i)) & (N - 1)].data.get()).assume_init_read()
                }));
            }
            let excess = items.len() - N;
            for &item in &items[..excess] {
                let _ = self.sink.get_mut().send(item);
            }
            head = head.wrapping_add(len);
            tail = head;
            self.head.store_mut(head);
            self.tail.store_mut(tail);
            &items[excess..]
        } else {
            items
        };

        // Evict to make room
        let len = tail.wrapping_sub(head);
        let free = N - len;
        if keep.len() > free {
            let evict_count = keep.len() - free;
            let h = head;
            // SAFETY: Slots [head..head+evict_count) are initialized. Each consumed once.
            let _ = self
                .sink
                .get_mut()
                .send_all((0..evict_count).map(|i| unsafe {
                    (*self.buffer[(h.wrapping_add(i)) & (N - 1)].data.get()).assume_init_read()
                }));
            self.head.store_mut(head.wrapping_add(evict_count));
        }

        // Bulk memcpy (at most 2 segments)
        let tail_idx = tail & (N - 1);
        let space_to_end = N - tail_idx;
        let count = keep.len();

        // SAFETY: Destination slots are uninitialized (evicted above or never written).
        // T: Copy, so memcpy is valid. Slot<T> is #[repr(transparent)] over
        // UnsafeCell<MaybeUninit<T>>, so data.get() yields a valid *mut T destination.
        // Source and destination do not overlap (stack slice vs ring buffer).
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

    /// Bulk-pop up to `buf.len()` items into a slice. Returns the count popped.
    ///
    /// Uses `memcpy` internally — at most two copies (from head to buffer end + wrap).
    #[inline]
    pub fn pop_slice(&mut self, buf: &mut [MaybeUninit<T>]) -> usize
    where
        T: Copy,
    {
        if buf.is_empty() {
            return 0;
        }

        let head = self.head.load_mut();
        let tail = self.tail.load_mut();
        let avail = tail.wrapping_sub(head);
        let count = buf.len().min(avail);
        if count == 0 {
            return 0;
        }

        let head_idx = head & (N - 1);
        let to_end = N - head_idx;

        // SAFETY: Slots [head..head+count) are initialized. T: Copy, so memcpy is
        // valid. Slot<T> is #[repr(transparent)], so data.get() yields valid *const T.
        // Source (ring buffer) and destination (caller's buf) do not overlap.
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

        self.head.store_mut(head.wrapping_add(count));
        count
    }

    /// Push an item then flush all to spout.
    #[inline]
    pub fn push_and_flush(&mut self, item: T) {
        self.push_mut(item);
        self.flush();
    }

    /// Flush all items to spout. Returns count flushed.
    ///
    /// Panic-safe: head is advanced after each item is sent, so a panic
    /// in the spout will not cause double-reads during drop.
    #[inline]
    pub fn flush(&mut self) -> usize {
        let head = self.head.load_mut();
        let tail = self.tail.load_mut();
        let count = tail.wrapping_sub(head);
        if count == 0 {
            return 0;
        }

        let sink = self.sink.get_mut();
        for i in 0..count {
            let idx = head.wrapping_add(i) & (N - 1);
            // SAFETY: Slot at idx is initialized (head..tail range). assume_init_read
            // moves the value out; head is advanced immediately after each send for
            // panic safety.
            let item = unsafe { (*self.buffer[idx].data.get()).assume_init_read() };
            self.head.store_mut(head.wrapping_add(i + 1));
            let _ = sink.send(item);
        }

        self.tail.store_mut(tail);
        count
    }

    /// Pop the oldest item.
    ///
    /// Uses interior mutability (`Cell`). Not thread-safe.
    #[inline]
    #[must_use]
    pub fn pop(&self) -> Option<T> {
        let head = self.head.load();
        let tail = self.tail.load();

        if head == tail {
            return None;
        }

        let idx = head & (N - 1);
        // SAFETY: Slot at head is initialized (head != tail). assume_init_read moves
        // value out. SpillRing is !Sync, so &self proves single-context access.
        let item = unsafe { (*self.buffer[idx].data.get()).assume_init_read() };
        self.head.store(head.wrapping_add(1));
        Some(item)
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

    /// Shared reference to the spout.
    ///
    /// # Safety contract
    /// Safe to call when no `push`, `flush`, or other mutation is in
    /// progress — guaranteed by Rust's borrow rules since `&self`
    /// prevents any concurrent `&mut self` call.
    #[inline]
    #[must_use]
    pub fn sink_ref(&self) -> &S {
        // SAFETY: SpillRing is !Sync, so &self proves single-context
        // access.  No &mut S alias can exist because that would
        // require &mut self, which conflicts with this &self borrow.
        unsafe { self.sink.get_ref() }
    }

    /// Reference to the spout.
    #[inline]
    #[must_use]
    pub fn sink(&mut self) -> &S {
        self.sink.get_mut()
    }

    /// Mutable reference to the spout.
    #[inline]
    pub fn sink_mut(&mut self) -> &mut S {
        self.sink.get_mut()
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
pub struct Drain<'a, T, const N: usize, S: Spout<T, Error = core::convert::Infallible>> {
    ring: &'a mut SpillRing<T, N, S>,
}

impl<T, const N: usize, S: Spout<T, Error = core::convert::Infallible>> Iterator
    for Drain<'_, T, N, S>
{
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

impl<T, const N: usize, S: Spout<T, Error = core::convert::Infallible>> ExactSizeIterator
    for Drain<'_, T, N, S>
{
}

impl<T, const N: usize, S: Spout<T, Error = core::convert::Infallible>> core::iter::Extend<T>
    for SpillRing<T, N, S>
{
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        for item in iter {
            self.push_mut(item);
        }
    }
}

impl<T, const N: usize> Default for SpillRing<T, N, DropSpout> {
    fn default() -> Self {
        Self::new()
    }
}

/// SpillRing can act as a Spout, enabling ring chaining (ring1 -> ring2).
impl<T, const N: usize, S: Spout<T, Error = core::convert::Infallible>> Spout<T>
    for SpillRing<T, N, S>
{
    type Error = core::convert::Infallible;

    #[inline]
    fn send(&mut self, item: T) -> Result<(), Self::Error> {
        self.push_mut(item);
        Ok(())
    }

    #[inline]
    fn flush(&mut self) -> Result<(), Self::Error> {
        SpillRing::flush(self);
        Ok(())
    }
}

/// On drop, flushes all buffered items to the spout, then flushes the spout itself.
///
/// When rings are chained (e.g. `ring1 → ring2 → ring3`), each ring's drop
/// triggers the next ring's `send` + `flush`, producing an O(depth) sequential
/// cascade. This is bounded by the number of rings the caller constructs and
/// is not a concern for typical usage (1–3 levels).
impl<T, const N: usize, S: Spout<T, Error = core::convert::Infallible>> Drop
    for SpillRing<T, N, S>
{
    fn drop(&mut self) {
        self.flush();
        let _ = self.sink.get_mut().flush();
    }
}

impl<T, const N: usize, S: Spout<T, Error = core::convert::Infallible>> RingInfo
    for SpillRing<T, N, S>
{
    #[inline]
    fn len(&self) -> usize {
        SpillRing::len(self)
    }

    #[inline]
    fn capacity(&self) -> usize {
        N
    }
}

impl<T, const N: usize, S: Spout<T, Error = core::convert::Infallible>> RingProducer<T>
    for SpillRing<T, N, S>
{
    #[inline]
    fn try_push(&mut self, item: T) -> Result<(), crate::PushError<T>> {
        let tail = self.tail.load_mut();
        let head = self.head.load_mut();

        if tail.wrapping_sub(head) >= N {
            return Err(crate::PushError::Full(item));
        }

        // SAFETY: Ring is not full (checked above). Slot at tail is uninitialized.
        // write() does not drop the previous value.
        unsafe {
            let slot = &self.buffer[tail & (N - 1)];
            (*slot.data.get()).write(item);
        }
        self.tail.store_mut(tail.wrapping_add(1));

        Ok(())
    }
}

impl<T, const N: usize, S: Spout<T, Error = core::convert::Infallible>> RingConsumer<T>
    for SpillRing<T, N, S>
{
    #[inline]
    fn try_pop(&mut self) -> Option<T> {
        self.pop_mut()
    }

    #[inline]
    fn peek(&mut self) -> Option<&T> {
        SpillRing::peek(self)
    }
}
