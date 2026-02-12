//! HashMap-based warm cache implementing [`WarmTier`].
//!
//! Items evicted from hot land here unserialized. They can be promoted
//! back for free (no deserialization). When full, the oldest item by
//! insertion timestamp overflows and must be serialized to cold storage.
//!
//! All operations are O(1) key-indexed via `HashMap`, except
//! `insert` at capacity which is O(n) to find the oldest entry.

use hashbrown::HashMap;

use crate::manager::traits::Checkpointable;

use super::WarmTier;

const DEFAULT_WARM_CAPACITY: usize = 64;

/// HashMap-based warm tier with timestamp-ordered overflow.
///
/// When used with `PebbleManager`, evicted items land here unserialized.
/// On overflow (capacity exceeded), the oldest entry is returned to the
/// caller for serialization to the cold tier.
pub struct WarmCache<T: Checkpointable> {
    entries: HashMap<T::Id, (T, u64)>,
    capacity: usize,
    counter: u64,
}

impl<T: Checkpointable> Default for WarmCache<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Checkpointable> WarmCache<T> {
    /// Create a warm cache with the default capacity (64).
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_WARM_CAPACITY)
    }

    /// Create a warm cache with the given capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: HashMap::with_capacity(capacity),
            capacity,
            counter: 0,
        }
    }

    /// Whether the warm cache is at capacity.
    #[inline]
    pub fn is_full(&self) -> bool {
        self.entries.len() >= self.capacity
    }

    /// Evict the oldest entry (smallest timestamp). Returns `None` if empty.
    fn evict_oldest(&mut self) -> Option<(T::Id, T)> {
        let oldest_id = self
            .entries
            .iter()
            .min_by_key(|(_id, (_item, ts))| *ts)
            .map(|(id, _)| *id)?;

        self.entries
            .remove(&oldest_id)
            .map(|(item, _ts)| (oldest_id, item))
    }
}

impl<T: Checkpointable> WarmTier<T> for WarmCache<T> {
    /// Insert an item into the warm cache.
    ///
    /// If `id` already exists, it is replaced (no overflow).
    /// If at capacity, the oldest entry is evicted and returned.
    fn insert(&mut self, id: T::Id, item: T) -> Option<(T::Id, T)> {
        // Replace existing entry — no overflow, just update timestamp.
        if self.entries.contains_key(&id) {
            self.counter = self.counter.wrapping_add(1);
            self.entries.insert(id, (item, self.counter));
            return None;
        }

        // Evict oldest if at capacity.
        let overflow = if self.entries.len() >= self.capacity {
            self.evict_oldest()
        } else {
            None
        };

        self.counter = self.counter.wrapping_add(1);
        self.entries.insert(id, (item, self.counter));
        overflow
    }

    #[inline]
    fn get(&self, id: T::Id) -> Option<&T> {
        self.entries.get(&id).map(|(item, _ts)| item)
    }

    #[inline]
    fn remove(&mut self, id: T::Id) -> Option<T> {
        self.entries.remove(&id).map(|(item, _ts)| item)
    }

    #[inline]
    fn contains(&self, id: T::Id) -> bool {
        self.entries.contains_key(&id)
    }

    fn drain(&mut self) -> impl Iterator<Item = (T::Id, T)> {
        self.entries.drain().map(|(id, (item, _ts))| (id, item))
    }

    #[inline]
    fn len(&self) -> usize {
        self.entries.len()
    }
}
