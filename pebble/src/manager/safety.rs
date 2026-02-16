//! Compile-time safety types for PebbleManager.
//!
//! [`CheckpointRef`] proves a checkpoint existed at creation time.
//! [`CapacityGuard`] proves the hot tier has a free slot.

use core::convert::Infallible;

use spout::Spout;

use super::cold::ColdTier;
use super::error::Result;
use super::manifest::ManifestEntry;
use super::pebble_manager::PebbleManager;
use super::traits::Checkpointable;
use super::warm::WarmTier;

/// Proof that a checkpoint existed in the manager at creation time.
///
/// Produced by [`PebbleManager::add_ref`], [`PebbleManager::insert_ref`],
/// and [`PebbleManager::locate`]. Consumed by [`PebbleManager::load_ref`]
/// and [`PebbleManager::rebuild_ref`] to skip the "does it exist?" check.
///
/// If the checkpoint was removed after the token was created, the consuming
/// method still returns an error — the token reduces the error surface but
/// cannot eliminate it entirely.
#[must_use = "token should be consumed by load_ref or rebuild_ref"]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CheckpointRef<Id> {
    id: Id,
}

impl<Id: Copy> CheckpointRef<Id> {
    /// The checkpoint ID this token refers to.
    #[inline]
    pub fn id(&self) -> Id {
        self.id
    }
}

impl<Id> CheckpointRef<Id> {
    pub(super) fn new(id: Id) -> Self {
        Self { id }
    }
}

/// Proof that the hot tier has at least one free slot.
///
/// Created by [`PebbleManager::ensure_capacity`], which evicts if necessary.
/// Consumed by [`CapacityGuard::add`] or [`CapacityGuard::insert`], which
/// skip the eviction check.
///
/// Borrows the manager mutably, so no other mutation can happen between
/// guard creation and use.
#[must_use = "guard should be consumed by .store() or .insert()"]
pub struct CapacityGuard<'a, T, C, W, S>
where
    T: Checkpointable,
    C: ColdTier<T>,
    W: WarmTier<T>,
    S: Spout<ManifestEntry<T::Id>, Error = Infallible>,
{
    manager: &'a mut PebbleManager<T, C, W, S>,
}

impl<'a, T, C, W, S> CapacityGuard<'a, T, C, W, S>
where
    T: Checkpointable,
    C: ColdTier<T>,
    W: WarmTier<T>,
    S: Spout<ManifestEntry<T::Id>, Error = Infallible>,
{
    pub(super) fn new(manager: &'a mut PebbleManager<T, C, W, S>) -> Self {
        Self { manager }
    }

    /// Store a checkpoint without eviction. The guard proves space exists.
    ///
    /// Consumes the guard. Returns a [`CheckpointRef`] for the new checkpoint.
    #[must_use = "this returns a Result that may indicate an error"]
    pub fn store(
        self,
        checkpoint: T,
        dependencies: &[T::Id],
    ) -> Result<CheckpointRef<T::Id>, T::Id, C::Error> {
        let mgr = self.manager;
        let state_id = checkpoint.checkpoint_id();

        mgr.dag.add_node(state_id, dependencies)?;
        mgr.register_hot(state_id, checkpoint);

        Ok(CheckpointRef::new(state_id))
    }

    /// Add a checkpoint using a deferred constructor. The guard proves space exists.
    ///
    /// Consumes the guard. Returns a [`CheckpointRef`] for the new checkpoint.
    #[must_use = "this returns a Result that may indicate an error"]
    pub fn insert<F>(
        self,
        dependencies: &[T::Id],
        constructor: F,
    ) -> Result<CheckpointRef<T::Id>, T::Id, C::Error>
    where
        F: FnOnce() -> T,
    {
        self.store(constructor(), dependencies)
    }
}
