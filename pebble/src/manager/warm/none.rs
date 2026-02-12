//! Zero-cost warm tier that compiles to nothing.

use crate::manager::traits::Checkpointable;

use super::WarmTier;

/// Zero-sized warm tier. Every insert overflows immediately.
///
/// When used with `PebbleManager`, evicted items skip the warm buffer
/// and go straight to the cold tier. The optimizer inlines all methods
/// and eliminates the warm-related branches entirely.
pub struct NoWarm;

impl<T: Checkpointable> WarmTier<T> for NoWarm {
    /// Always returns the item back as overflow — no buffering.
    #[inline(always)]
    fn insert(&mut self, id: T::Id, item: T) -> Option<(T::Id, T)> {
        Some((id, item))
    }

    /// Always `None` — nothing is buffered.
    #[inline(always)]
    fn get(&self, _id: T::Id) -> Option<&T> {
        None
    }

    /// Always `None` — nothing to remove.
    #[inline(always)]
    fn remove(&mut self, _id: T::Id) -> Option<T> {
        None
    }

    /// Always `false`.
    #[inline(always)]
    fn contains(&self, _id: T::Id) -> bool {
        false
    }

    /// Empty iterator.
    #[inline(always)]
    fn drain(&mut self) -> impl Iterator<Item = (T::Id, T)> {
        core::iter::empty()
    }

    /// Always `0`.
    #[inline(always)]
    fn len(&self) -> usize {
        0
    }
}
