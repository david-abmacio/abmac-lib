//! Iterators for `SpillRing`.

use crate::ring::SpillRing;
use spout::Spout;

/// Immutable iterator.
///
/// Holds `&'a mut SpillRing` to prevent `push(&self)` from invalidating
/// yielded references. The mutable borrow is purely for exclusivity —
/// the iterator only reads slot contents.
pub struct SpillRingIter<'a, T, const N: usize, S: Spout<T, Error = core::convert::Infallible>> {
    ring: &'a SpillRing<T, N, S>,
    pos: usize,
    len: usize,
    head: usize,
}

impl<'a, T, const N: usize, S: Spout<T, Error = core::convert::Infallible>>
    SpillRingIter<'a, T, N, S>
{
    pub(crate) fn new(ring: &'a mut SpillRing<T, N, S>) -> Self {
        let len = ring.len();
        let head = ring.head.load();
        Self {
            ring,
            pos: 0,
            len,
            head,
        }
    }
}

impl<'a, T, const N: usize, S: Spout<T, Error = core::convert::Infallible>> Iterator
    for SpillRingIter<'a, T, N, S>
{
    type Item = &'a T;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.len {
            return None;
        }
        let idx = self.head.wrapping_add(self.pos) & (N - 1);
        self.pos += 1;
        // SAFETY: The &mut self borrow in new() prevents push(&self) from
        // invalidating slot contents. Slot at idx is initialized
        // (pos < len, where len = tail - head at construction).
        Some(unsafe {
            let slot = &self.ring.buffer[idx];
            (*slot.data.get()).assume_init_ref()
        })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.len - self.pos;
        (remaining, Some(remaining))
    }

    #[inline]
    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        if n >= self.len - self.pos {
            self.pos = self.len;
            return None;
        }
        self.pos += n;
        self.next()
    }
}

impl<T, const N: usize, S: Spout<T, Error = core::convert::Infallible>> ExactSizeIterator
    for SpillRingIter<'_, T, N, S>
{
}
impl<T, const N: usize, S: Spout<T, Error = core::convert::Infallible>> core::iter::FusedIterator
    for SpillRingIter<'_, T, N, S>
{
}

/// Mutable iterator.
///
/// Stores a raw pointer to avoid creating `&mut T` through a shared reference.
/// The lifetime `'a` is enforced via `PhantomData`.
pub struct SpillRingIterMut<'a, T, const N: usize, S: Spout<T, Error = core::convert::Infallible>> {
    ring: *mut SpillRing<T, N, S>,
    pos: usize,
    len: usize,
    head: usize,
    _marker: core::marker::PhantomData<&'a mut SpillRing<T, N, S>>,
}

impl<'a, T, const N: usize, S: Spout<T, Error = core::convert::Infallible>>
    SpillRingIterMut<'a, T, N, S>
{
    pub(crate) fn new(ring: &'a mut SpillRing<T, N, S>) -> Self {
        let len = ring.len();
        let head = ring.head.load();
        Self {
            ring: core::ptr::from_mut(ring),
            pos: 0,
            len,
            head,
            _marker: core::marker::PhantomData,
        }
    }
}

impl<'a, T, const N: usize, S: Spout<T, Error = core::convert::Infallible>> Iterator
    for SpillRingIterMut<'a, T, N, S>
{
    type Item = &'a mut T;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.len {
            return None;
        }
        let idx = self.head.wrapping_add(self.pos) & (N - 1);
        self.pos += 1;
        // SAFETY: `ring` is a valid pointer derived from `&mut SpillRing` in `new()`.
        // Each index is yielded exactly once (pos increments), so no aliasing occurs.
        // The `UnsafeCell` in each slot permits mutable access through a raw pointer.
        Some(unsafe {
            let slot = &(*self.ring).buffer[idx];
            &mut *(*slot.data.get()).as_mut_ptr()
        })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.len - self.pos;
        (remaining, Some(remaining))
    }
}

impl<T, const N: usize, S: Spout<T, Error = core::convert::Infallible>> ExactSizeIterator
    for SpillRingIterMut<'_, T, N, S>
{
}
impl<T, const N: usize, S: Spout<T, Error = core::convert::Infallible>> core::iter::FusedIterator
    for SpillRingIterMut<'_, T, N, S>
{
}

impl<'a, T, const N: usize, S: Spout<T, Error = core::convert::Infallible>> IntoIterator
    for &'a mut SpillRing<T, N, S>
{
    type Item = &'a mut T;
    type IntoIter = SpillRingIterMut<'a, T, N, S>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        SpillRingIterMut::new(self)
    }
}
