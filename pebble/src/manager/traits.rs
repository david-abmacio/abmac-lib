//! Traits for checkpointable types.

use core::hash::Hash;
use hashbrown::HashMap;

/// A type that can be checkpointed and rebuilt from dependencies.
pub trait Checkpointable: Sized + Clone {
    /// Unique identifier type (e.g., `u64`, `u128`, UUID).
    type Id: Copy + Eq + Hash + Default + core::fmt::Debug;

    /// Error type for rebuild failures.
    type RebuildError: core::fmt::Debug;

    /// Get the checkpoint's unique identifier.
    fn checkpoint_id(&self) -> Self::Id;

    /// Rebuild this checkpoint from its dependencies.
    ///
    /// Called during state rebuild when a checkpoint is loaded from storage.
    /// Implementors with no dependencies should return `Ok(base)`.
    fn compute_from_dependencies(
        base: Self,
        deps: &HashMap<Self::Id, &Self>,
    ) -> core::result::Result<Self, Self::RebuildError>;
}
