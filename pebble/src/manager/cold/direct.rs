//! Direct (unbuffered) cold tier implementation.

use alloc::vec::Vec;
use core::fmt;
use core::hash::Hash;

use spout::Spout;

pub use crate::errors::cold::DirectStorageError;
use crate::manager::traits::Checkpointable;
use crate::storage::{
    CheckpointLoader, CheckpointMetadata, CheckpointRemover, RecoverableStorage, SessionId,
};
use bytecast::ByteSerializer;

use super::{ColdTier, RecoverableColdTier};

/// Unbuffered cold tier: serialize and send immediately.
///
/// `DirectStorage` pairs a storage backend with bytecast serialization.
/// Every `store()` call serializes the checkpoint and sends it to storage
/// in one step — no ring buffer, no batching. `buffered_count()` is
/// always zero.
///
/// Always available (no feature gate required).
///
/// # Type Parameters
/// - `S` — Storage backend implementing [`Spout`] (for writes) and
///   [`CheckpointLoader`] (for reads)
pub struct DirectStorage<S> {
    storage: S,
}

impl<S> DirectStorage<S> {
    /// Create a new direct storage tier.
    pub fn new(storage: S) -> Self {
        Self { storage }
    }

    /// Borrow the underlying storage.
    pub fn storage(&self) -> &S {
        &self.storage
    }

    /// Mutably borrow the underlying storage.
    pub fn storage_mut(&mut self) -> &mut S {
        &mut self.storage
    }
}

/// Debug-only file-backed cold tier for development testing.
///
/// Persists checkpoints to `pebble_debug/` in the system temp dir.
/// Does not exist in release builds — forces choosing a real backend.
#[cfg(all(debug_assertions, feature = "std"))]
impl DirectStorage<crate::storage::DebugFileStorage> {
    /// Create a debug cold tier backed by temp files.
    pub fn debug() -> Self {
        Self::new(crate::storage::DebugFileStorage::new())
    }
}

/// Debug-only in-memory cold tier for development testing.
///
/// Falls back to in-memory storage when `std` is not available.
/// Does not exist in release builds — forces choosing a real backend.
#[cfg(all(debug_assertions, not(feature = "std")))]
impl DirectStorage<crate::storage::InMemoryStorage> {
    /// Create a debug cold tier backed by in-memory storage.
    pub fn debug() -> Self {
        Self::new(crate::storage::InMemoryStorage::new())
    }
}

impl<T, S> ColdTier<T> for DirectStorage<S>
where
    T: Checkpointable + bytecast::ToBytes + bytecast::FromBytes,
    S: Spout<(T::Id, Vec<u8>, Vec<T::Id>)> + CheckpointLoader<T::Id> + CheckpointRemover<T::Id>,
{
    type Error = DirectStorageError;

    fn store(&mut self, id: T::Id, checkpoint: &T, deps: &[T::Id]) -> Result<(), Self::Error> {
        let bytes = ByteSerializer
            .serialize(checkpoint)
            .map_err(|source| DirectStorageError::Serializer { source })?;
        let _ = self.storage.send((id, bytes, deps.to_vec()));
        Ok(())
    }

    fn load(&self, id: T::Id) -> Result<T, Self::Error> {
        let bytes = self.storage.load(id)?;
        ByteSerializer
            .deserialize(&bytes)
            .map_err(|source| DirectStorageError::Serializer { source })
    }

    fn contains(&self, id: T::Id) -> bool {
        self.storage.contains(id)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        let _ = self.storage.flush();
        Ok(())
    }

    fn remove(&mut self, id: T::Id) -> Result<bool, Self::Error> {
        Ok(self.storage.remove(id))
    }

    fn buffered_count(&self) -> usize {
        0
    }
}

impl<T, S, SId, const MAX_DEPS: usize> RecoverableColdTier<T, SId, MAX_DEPS> for DirectStorage<S>
where
    T: Checkpointable + bytecast::ToBytes + bytecast::FromBytes,
    T::Id: Hash,
    S: Spout<(T::Id, Vec<u8>, Vec<T::Id>)>
        + RecoverableStorage<T::Id, SId, MAX_DEPS>
        + CheckpointRemover<T::Id>,
    SId: SessionId,
{
    type MetadataIter<'a>
        = MetadataIter<'a, T::Id, S, SId, MAX_DEPS>
    where
        Self: 'a,
        T::Id: 'a,
        SId: 'a;

    fn iter_metadata(&self) -> Self::MetadataIter<'_> {
        MetadataIter {
            inner: self.storage.iter_metadata(),
        }
    }

    fn get_metadata(&self, id: T::Id) -> Option<CheckpointMetadata<T::Id, SId, MAX_DEPS>> {
        self.storage.get_metadata(id)
    }
}

/// Iterator adapter for [`DirectStorage::iter_metadata`].
pub struct MetadataIter<'a, CId, S, SId, const MAX_DEPS: usize>
where
    CId: Copy + Eq + Hash + Default + fmt::Debug + 'a,
    S: RecoverableStorage<CId, SId, MAX_DEPS> + 'a,
    SId: SessionId + 'a,
{
    inner: S::MetadataIter<'a>,
}

impl<'a, CId, S, SId, const MAX_DEPS: usize> Iterator for MetadataIter<'a, CId, S, SId, MAX_DEPS>
where
    CId: Copy + Eq + Hash + Default + fmt::Debug,
    S: RecoverableStorage<CId, SId, MAX_DEPS>,
    SId: SessionId,
{
    type Item = (CId, CheckpointMetadata<CId, SId, MAX_DEPS>);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}
