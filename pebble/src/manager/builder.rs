//! Builder for PebbleManager.

extern crate alloc;

use crate::strategy::Strategy;

use super::cold::ColdTier;
use super::pebble_manager::PebbleManager;
use super::traits::Checkpointable;
use super::warm::WarmTier;

/// Default hot tier capacity when no hint is provided.
const DEFAULT_HOT_CAPACITY: usize = 16;

/// Default ring buffer capacity for the convenience `.storage()` path.
#[cfg(feature = "bytecast")]
const DEFAULT_RING_BUFFER_CAPACITY: usize = 64;

/// Builder for [`PebbleManager`] with sensible defaults.
///
/// The simplest path uses [`.storage()`](PebbleManagerBuilder::storage) with
/// the `bytecast` feature, which wires up serialization and tiers automatically.
/// For custom serializers or tier types, use `.cold()` and `.warm()` instead.
///
/// # Example
///
/// ```
/// use pebble::{PebbleManagerBuilder, InMemoryStorage, DirectStorage, NoWarm};
/// # use pebble::{Checkpointable, CheckpointSerializer};
/// # #[derive(Clone)] struct MyCp { id: u64 }
/// # impl Checkpointable for MyCp {
/// #     type Id = u64;
/// #     type RebuildError = ();
/// #     fn checkpoint_id(&self) -> u64 { self.id }
/// #     fn dependencies(&self) -> &[u64] { &[] }
/// #     fn compute_from_dependencies(
/// #         base: Self, _deps: &hashbrown::HashMap<Self::Id, &Self>,
/// #     ) -> Result<Self, Self::RebuildError> { Ok(base) }
/// # }
/// # struct MySer;
/// # impl CheckpointSerializer<MyCp> for MySer {
/// #     type Error = &'static str;
/// #     fn serialize(&self, _: &MyCp) -> Result<Vec<u8>, &'static str> { Ok(vec![]) }
/// #     fn deserialize(&self, _: &[u8]) -> Result<MyCp, &'static str> { Ok(MyCp { id: 0 }) }
/// # }
///
/// let cold = DirectStorage::new(InMemoryStorage::<u64, u128, 8>::new(), MySer);
/// let manager = PebbleManagerBuilder::new()
///     .cold(cold)
///     .warm(NoWarm)
///     .hot_capacity(4)
///     .build::<MyCp>()
///     .unwrap();
/// ```
pub struct PebbleManagerBuilder<C = (), W = ()> {
    cold: C,
    warm: W,
    strategy: Strategy,
    hot_capacity: usize,
}

impl PebbleManagerBuilder {
    /// Create a new builder with defaults.
    pub fn new() -> Self {
        Self {
            cold: (),
            warm: (),
            strategy: Strategy::default(),
            hot_capacity: DEFAULT_HOT_CAPACITY,
        }
    }
}

impl Default for PebbleManagerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "bytecast")]
impl PebbleManagerBuilder<(), ()> {
    /// Provide a storage backend and let the builder wire up serialization
    /// and tier configuration automatically.
    ///
    /// Uses `BytecastSerializer` for serialization. When `cold-buffer` is
    /// enabled, builds a `RingCold` + `WarmCache` configuration. Otherwise
    /// falls back to `DirectStorage` + `NoWarm`.
    ///
    /// For custom serializers or tier types, use `.cold()` and `.warm()` instead.
    pub fn storage<S>(self, storage: S) -> StorageBuilder<S, DEFAULT_RING_BUFFER_CAPACITY> {
        StorageBuilder {
            storage,
            strategy: self.strategy,
            hot_capacity: self.hot_capacity,
            #[cfg(feature = "cold-buffer")]
            warm_capacity: None,
        }
    }
}

impl<W> PebbleManagerBuilder<(), W> {
    /// Set the cold tier (required).
    pub fn cold<C>(self, cold: C) -> PebbleManagerBuilder<C, W> {
        PebbleManagerBuilder {
            cold,
            warm: self.warm,
            strategy: self.strategy,
            hot_capacity: self.hot_capacity,
        }
    }
}

impl<C> PebbleManagerBuilder<C, ()> {
    /// Set the warm tier (required).
    pub fn warm<W>(self, warm: W) -> PebbleManagerBuilder<C, W> {
        PebbleManagerBuilder {
            cold: self.cold,
            warm,
            strategy: self.strategy,
            hot_capacity: self.hot_capacity,
        }
    }
}

impl<C, W> PebbleManagerBuilder<C, W> {
    /// Set the eviction strategy. Default: `Strategy::default()` (DAG).
    pub fn strategy(mut self, strategy: Strategy) -> Self {
        self.strategy = strategy;
        self
    }

    /// Set hot tier capacity (max red pebbles). Default: 16.
    pub fn hot_capacity(mut self, size: usize) -> Self {
        self.hot_capacity = size;
        self
    }

    /// Compute `hot_capacity` from expected total checkpoints.
    ///
    /// Sets `hot_capacity = sqrt(n)` (minimum 1). Users who know their
    /// workload size get optimal configuration without understanding pebble
    /// game theory.
    pub fn hint_total_checkpoints(mut self, n: usize) -> Self {
        self.hot_capacity = n.isqrt().max(1);
        self
    }

    /// Build the [`PebbleManager`].
    ///
    /// Returns `Err` if `hot_capacity` is 0.
    ///
    /// `T` is the checkpoint type.
    pub fn build<T>(
        self,
    ) -> core::result::Result<PebbleManager<T, C, W>, super::error::BuilderError>
    where
        T: Checkpointable,
        C: ColdTier<T>,
        W: WarmTier<T>,
    {
        if self.hot_capacity == 0 {
            return Err(super::error::BuilderError::ZeroHotCapacity);
        }

        Ok(PebbleManager::new(
            self.cold,
            self.warm,
            self.strategy,
            self.hot_capacity,
        ))
    }
}

/// Builder that defers tier construction until `.build()` when `T` is known.
///
/// Created by [`PebbleManagerBuilder::storage`]. Requires the `bytecast` feature.
///
/// The const generic `N` controls the write buffer (ring) capacity when
/// `cold-buffer` is enabled. Defaults to 64. To change it:
///
/// ```text
/// PebbleManagerBuilder::new()
///     .storage(my_storage)
///     .ring_capacity::<32>()
///     .build::<MyCheckpoint>()
/// ```
#[cfg(feature = "bytecast")]
pub struct StorageBuilder<S, const N: usize = DEFAULT_RING_BUFFER_CAPACITY> {
    storage: S,
    strategy: Strategy,
    hot_capacity: usize,
    #[cfg(feature = "cold-buffer")]
    warm_capacity: Option<usize>,
}

#[cfg(feature = "bytecast")]
impl<S, const N: usize> StorageBuilder<S, N> {
    /// Change the write buffer (ring) capacity.
    ///
    /// Only affects the `cold-buffer` build path. Default: 64.
    pub fn ring_capacity<const M: usize>(self) -> StorageBuilder<S, M> {
        StorageBuilder {
            storage: self.storage,
            strategy: self.strategy,
            hot_capacity: self.hot_capacity,
            #[cfg(feature = "cold-buffer")]
            warm_capacity: self.warm_capacity,
        }
    }

    /// Set the eviction strategy. Default: `Strategy::default()` (DAG).
    pub fn strategy(mut self, strategy: Strategy) -> Self {
        self.strategy = strategy;
        self
    }

    /// Set hot tier capacity (max red pebbles). Default: 16.
    pub fn hot_capacity(mut self, size: usize) -> Self {
        self.hot_capacity = size;
        self
    }

    /// Compute `hot_capacity` from expected total checkpoints.
    ///
    /// Sets `hot_capacity = sqrt(n)` (minimum 1).
    pub fn hint_total_checkpoints(mut self, n: usize) -> Self {
        self.hot_capacity = n.isqrt().max(1);
        self
    }

    /// Set warm cache capacity. Default: matches `hot_capacity`.
    #[cfg(feature = "cold-buffer")]
    pub fn warm_capacity(mut self, capacity: usize) -> Self {
        self.warm_capacity = Some(capacity);
        self
    }
}

/// A `PebbleManager` wired to `RingCold` + `WarmCache` via `BytecastSerializer`.
#[cfg(all(feature = "bytecast", feature = "cold-buffer"))]
pub type BufferedManager<T, S, const N: usize> = PebbleManager<
    T,
    super::cold::RingCold<<T as Checkpointable>::Id, S, super::BytecastSerializer, N>,
    super::warm::WarmCache<T>,
>;

#[cfg(all(feature = "bytecast", feature = "cold-buffer"))]
impl<S, const N: usize> StorageBuilder<S, N> {
    /// Build a [`PebbleManager`] with `RingCold` and `WarmCache`.
    pub fn build<T>(
        self,
    ) -> core::result::Result<BufferedManager<T, S, N>, super::error::BuilderError>
    where
        T: Checkpointable + bytecast::ToBytes + bytecast::FromBytes,
        T::Id: Copy,
        S: spout::Spout<(T::Id, alloc::vec::Vec<u8>)> + crate::storage::CheckpointLoader<T::Id>,
    {
        if self.hot_capacity == 0 {
            return Err(super::error::BuilderError::ZeroHotCapacity);
        }

        let cold = super::cold::RingCold::with_storage(self.storage);
        let warm_cap = self.warm_capacity.unwrap_or(self.hot_capacity);
        debug_assert!(
            warm_cap <= self.hot_capacity,
            "warm_capacity ({warm_cap}) should not exceed hot_capacity ({}); \
             increase hot_capacity instead",
            self.hot_capacity,
        );
        let warm = super::warm::WarmCache::with_capacity(warm_cap);

        Ok(PebbleManager::new(
            cold,
            warm,
            self.strategy,
            self.hot_capacity,
        ))
    }
}

#[cfg(all(feature = "bytecast", not(feature = "cold-buffer")))]
impl<S, const N: usize> StorageBuilder<S, N> {
    /// Build a [`PebbleManager`] with `DirectStorage` and `NoWarm`.
    pub fn build<T>(
        self,
    ) -> core::result::Result<
        PebbleManager<
            T,
            super::cold::DirectStorage<S, super::BytecastSerializer>,
            super::warm::NoWarm,
        >,
        super::error::BuilderError,
    >
    where
        T: Checkpointable + bytecast::ToBytes + bytecast::FromBytes,
        S: spout::Spout<(T::Id, alloc::vec::Vec<u8>)> + crate::storage::CheckpointLoader<T::Id>,
    {
        if self.hot_capacity == 0 {
            return Err(super::error::BuilderError::ZeroHotCapacity);
        }

        let cold = super::cold::DirectStorage::with_storage(self.storage);

        Ok(PebbleManager::new(
            cold,
            super::warm::NoWarm,
            self.strategy,
            self.hot_capacity,
        ))
    }
}
