//! Direct (unbuffered) cold tier implementation.

use alloc::vec::Vec;
use core::fmt;
use core::hash::Hash;

use spout::Spout;

use crate::manager::traits::{CheckpointSerializer, Checkpointable};
use crate::storage::{
    CheckpointLoader, CheckpointMetadata, RecoverableStorage, SessionId, StorageError,
};

use super::{ColdTier, RecoverableColdTier};

/// Unbuffered cold tier: serialize and send immediately.
///
/// `DirectStorage` pairs a storage backend with a serializer. Every
/// `store()` call serializes the checkpoint and sends it to storage
/// in one step — no ring buffer, no batching. `buffered_count()` is
/// always zero.
///
/// Always available (no feature gate required).
///
/// # Type Parameters
/// - `S` — Storage backend implementing [`Spout`] (for writes) and
///   [`CheckpointLoader`] (for reads)
/// - `Ser` — Checkpoint serializer
pub struct DirectStorage<S, Ser> {
    storage: S,
    serializer: Ser,
}

impl<S, Ser> DirectStorage<S, Ser> {
    /// Create a new direct storage tier with a custom serializer.
    pub fn new(storage: S, serializer: Ser) -> Self {
        Self {
            storage,
            serializer,
        }
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

#[cfg(feature = "bytecast")]
impl<S> DirectStorage<S, super::super::BytecastSerializer> {
    /// Create a new direct storage tier using `BytecastSerializer`.
    pub fn with_storage(storage: S) -> Self {
        Self::new(storage, super::super::BytecastSerializer)
    }
}

/// Error type for [`DirectStorage`] operations.
///
/// Wraps either a serialization/deserialization error or a storage error.
pub enum DirectStorageError<SerErr: fmt::Debug + fmt::Display> {
    /// Serialization or deserialization failed.
    Serializer(SerErr),
    /// Storage operation failed.
    Storage(StorageError),
}

impl<SerErr: fmt::Debug + fmt::Display> fmt::Debug for DirectStorageError<SerErr> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Serializer(e) => write!(f, "Serializer({e:?})"),
            Self::Storage(e) => write!(f, "Storage({e:?})"),
        }
    }
}

impl<SerErr: fmt::Debug + fmt::Display> fmt::Display for DirectStorageError<SerErr> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Serializer(e) => write!(f, "serializer error: {e}"),
            Self::Storage(e) => write!(f, "storage error: {e}"),
        }
    }
}

impl<SerErr: fmt::Debug + fmt::Display> From<StorageError> for DirectStorageError<SerErr> {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

impl<T, S, Ser> ColdTier<T> for DirectStorage<S, Ser>
where
    T: Checkpointable,
    S: Spout<(T::Id, Vec<u8>)> + CheckpointLoader<T::Id>,
    Ser: CheckpointSerializer<T>,
{
    type Error = DirectStorageError<Ser::Error>;

    fn store(&mut self, id: T::Id, checkpoint: &T) -> Result<(), Self::Error> {
        let bytes = self
            .serializer
            .serialize(checkpoint)
            .map_err(DirectStorageError::Serializer)?;
        self.storage.send((id, bytes));
        Ok(())
    }

    fn load(&self, id: T::Id) -> Result<T, Self::Error> {
        let bytes = self.storage.load(id)?;
        self.serializer
            .deserialize(&bytes)
            .map_err(DirectStorageError::Serializer)
    }

    fn contains(&self, id: T::Id) -> bool {
        self.storage.contains(id)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.storage.flush();
        Ok(())
    }

    fn buffered_count(&self) -> usize {
        0
    }
}

impl<T, S, Ser, SId, const MAX_DEPS: usize> RecoverableColdTier<T, SId, MAX_DEPS>
    for DirectStorage<S, Ser>
where
    T: Checkpointable,
    T::Id: Hash,
    S: Spout<(T::Id, Vec<u8>)> + RecoverableStorage<T::Id, SId, MAX_DEPS>,
    Ser: CheckpointSerializer<T>,
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
