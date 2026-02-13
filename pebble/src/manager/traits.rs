//! Traits for checkpointable types and serialization.

use alloc::vec::Vec;
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

/// Serialize and deserialize checkpoints.
pub trait CheckpointSerializer<T> {
    /// Error type for serialization failures.
    type Error: core::fmt::Debug + core::fmt::Display;

    /// Serialize a checkpoint to bytes.
    fn serialize(&self, value: &T) -> core::result::Result<Vec<u8>, Self::Error>;

    /// Deserialize a checkpoint from bytes.
    fn deserialize(&self, bytes: &[u8]) -> core::result::Result<T, Self::Error>;
}
