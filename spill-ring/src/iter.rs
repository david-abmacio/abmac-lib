//! Iterators for SpillRing.

use crate::ring::SpillRing;
use spout::Spout;

/// Immutable iterator.
pub struct SpillRingIter<'a, T, const N: usize, S: Spout<T>> {
    ring: &'a SpillRing<T, N, S>,
    pos: usize,
    len: usize,
    head: usize,
}

impl<'a, T, const N: usize, S: Spout<T>> SpillRingIter<'a, T, N, S> {
    pub(crate) fn new(ring: &'a SpillRing<T, N, S>) -> Self {
        Self {
            ring,
            pos: 0,
            len: ring.len(),
            head: ring.head.load(),
        }
    }
}

impl<'a, T, const N: usize, S: Spout<T>> Iterator for SpillRingIter<'a, T, N, S> {
    type Item = &'a T;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.len {
            return None;
        }
        let idx = self.head.wrapping_add(self.pos) & (N - 1);
        self.pos += 1;
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

impl<T, const N: usize, S: Spout<T>> ExactSizeIterator for SpillRingIter<'_, T, N, S> {}
impl<T, const N: usize, S: Spout<T>> core::iter::FusedIterator for SpillRingIter<'_, T, N, S> {}

/// Mutable iterator.
pub struct SpillRingIterMut<'a, T, const N: usize, S: Spout<T>> {
    ring: &'a SpillRing<T, N, S>,
    pos: usize,
    len: usize,
    head: usize,
}

impl<'a, T, const N: usize, S: Spout<T>> SpillRingIterMut<'a, T, N, S> {
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

impl<'a, T, const N: usize, S: Spout<T>> Iterator for SpillRingIterMut<'a, T, N, S> {
    type Item = &'a mut T;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.len {
            return None;
        }
        let idx = self.head.wrapping_add(self.pos) & (N - 1);
        self.pos += 1;
        Some(unsafe {
            let slot = &self.ring.buffer[idx];
            &mut *(*slot.data.get()).as_mut_ptr()
        })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.len - self.pos;
        (remaining, Some(remaining))
    }
}

impl<T, const N: usize, S: Spout<T>> ExactSizeIterator for SpillRingIterMut<'_, T, N, S> {}
impl<T, const N: usize, S: Spout<T>> core::iter::FusedIterator for SpillRingIterMut<'_, T, N, S> {}
