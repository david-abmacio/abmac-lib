//! Read accessors for `SpillRing`.
//!
//! `SpillRing` is `!Sync`, so `&self` receivers are safe — no concurrent
//! producer can invalidate references.

use crate::iter::SpillRingIter;
use crate::ring::SpillRing;
use spout::Spout;

impl<T, const N: usize, S: Spout<T, Error = core::convert::Infallible>> SpillRing<T, N, S> {
    /// Peek at the oldest item.
    #[inline]
    #[must_use]
    pub fn peek(&self) -> Option<&T> {
        let head = self.head.load();
        let tail = self.tail.load();
        if head == tail {
            return None;
        }
        // SAFETY: Slot at head is initialized (head != tail). SpillRing is !Sync,
        // so &self proves single-context access — no concurrent mutation.
        Some(unsafe {
            let slot = &self.buffer[head & (N - 1)];
            (*slot.data.get()).assume_init_ref()
        })
    }

    /// Peek at the newest item.
    #[inline]
    #[must_use]
    pub fn peek_back(&self) -> Option<&T> {
        let head = self.head.load();
        let tail = self.tail.load();
        if head == tail {
            return None;
        }
        let idx = tail.wrapping_sub(1) & (N - 1);
        // SAFETY: Slot at tail-1 is initialized (head != tail). SpillRing is !Sync,
        // so &self proves single-context access.
        Some(unsafe {
            let slot = &self.buffer[idx];
            (*slot.data.get()).assume_init_ref()
        })
    }

    /// Get item by index (0 = oldest).
    #[inline]
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&T> {
        let head = self.head.load();
        let tail = self.tail.load();
        let len = tail.wrapping_sub(head);
        if index >= len {
            return None;
        }
        let idx = head.wrapping_add(index) & (N - 1);
        // SAFETY: Slot at head+index is initialized (index < len). SpillRing is
        // !Sync, so &self proves single-context access.
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
}
