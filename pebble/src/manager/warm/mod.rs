//! Warm-tier eviction buffer trait and implementations.

mod cache;
mod none;

use super::traits::Checkpointable;

pub use cache::WarmCache;
pub use none::NoWarm;

/// Warm-side eviction buffer.
///
/// Holds unserialized checkpoints between hot and cold tiers. Items
/// evicted from hot land here. They can be promoted back to hot for
/// free (no deserialization). When full, the oldest item overflows
/// and must be serialized to cold.
///
/// Two implementations:
/// - [`NoWarm`] — ZST, compiles to nothing
/// - [`WarmCache`] — HashMap-based, timestamp-ordered overflow
pub trait WarmTier<T: Checkpointable> {
    /// Insert an item. Returns overflow (oldest) if at capacity.
    fn insert(&mut self, id: T::Id, item: T) -> Option<(T::Id, T)>;

    /// Get a reference without promoting.
    fn get(&self, id: T::Id) -> Option<&T>;

    /// Remove and return (promote to hot).
    fn remove(&mut self, id: T::Id) -> Option<T>;

    /// Check membership.
    fn contains(&self, id: T::Id) -> bool;

    /// Drain all items.
    fn drain(&mut self) -> impl Iterator<Item = (T::Id, T)>;

    /// Number of items.
    fn len(&self) -> usize;

    /// Whether empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
