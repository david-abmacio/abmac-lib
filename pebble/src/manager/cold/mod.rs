//! Cold-tier storage trait and implementations.

mod direct;
#[cfg(feature = "std")]
mod parallel;
mod ring;

use super::traits::Checkpointable;
use crate::storage::{CheckpointMetadata, SessionId};

pub use direct::{DirectStorage, DirectStorageError};
#[cfg(feature = "std")]
pub use parallel::ParallelCold;
pub use ring::RingCold;

/// Debug-only cold tier type alias.
///
/// Resolves to file-backed storage (with `std`) or in-memory
/// storage (without). Does not exist in release builds.
#[cfg(all(debug_assertions, feature = "std"))]
pub type DebugCold = DirectStorage<crate::storage::DebugFileStorage>;

/// Debug-only cold tier type alias.
///
/// Falls back to in-memory storage when `std` is not available.
/// Does not exist in release builds.
#[cfg(all(debug_assertions, not(feature = "std")))]
pub type DebugCold = DirectStorage<crate::storage::InMemoryStorage>;

/// Cold-side storage abstraction.
///
/// Owns the serializer and storage backend. PebbleManager hands it
/// unserialized checkpoints; the cold tier handles serialization,
/// buffering, and delivery to storage.
///
/// Three implementations:
/// - [`DirectStorage`] — serialize and send immediately, no buffering (always available)
/// - [`RingCold`] — serialize into a SpillRing (always available)
/// - [`ParallelCold`] — parallel I/O across a WorkerPool (requires `std`)
pub trait ColdTier<T: Checkpointable> {
    /// Error type for storage operations.
    type Error: core::fmt::Debug + core::fmt::Display;

    /// Serialize and store a checkpoint.
    fn store(&mut self, id: T::Id, checkpoint: &T) -> Result<(), Self::Error>;

    /// Load and deserialize a checkpoint from storage.
    ///
    /// Items that have been [`store`](Self::store)d but not yet
    /// [`flush`](Self::flush)ed may not be loadable. Callers should
    /// flush before loading if visibility of recent stores is needed.
    fn load(&self, id: T::Id) -> Result<T, Self::Error>;

    /// Check if a checkpoint exists in storage.
    ///
    /// Like [`load`](Self::load), this only queries committed storage.
    /// Buffered items may not be visible until [`flush`](Self::flush).
    fn contains(&self, id: T::Id) -> bool;

    /// Flush all buffered items to storage, making them visible to
    /// [`load`](Self::load) and [`contains`](Self::contains).
    fn flush(&mut self) -> Result<(), Self::Error>;

    /// Number of items currently buffered (not yet in storage).
    fn buffered_count(&self) -> usize;
}

/// Extension for recovery from existing storage.
///
/// Cold tier implementations that support this can be used with
/// `PebbleManager::recover()` to rebuild state from persisted checkpoints.
pub trait RecoverableColdTier<T: Checkpointable, SId: SessionId = u128, const MAX_DEPS: usize = 8>:
    ColdTier<T>
{
    /// Iterator over all checkpoint metadata.
    type MetadataIter<'a>: Iterator<Item = (T::Id, CheckpointMetadata<T::Id, SId, MAX_DEPS>)>
    where
        Self: 'a,
        T::Id: 'a,
        SId: 'a;

    /// Iterate all checkpoint metadata for discovery.
    fn iter_metadata(&self) -> Self::MetadataIter<'_>;

    /// Get metadata for a specific checkpoint.
    fn get_metadata(&self, id: T::Id) -> Option<CheckpointMetadata<T::Id, SId, MAX_DEPS>>;
}
