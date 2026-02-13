//! SpillRing-backed cold tier implementation.
//!
//! Serialized checkpoints are pushed into a fixed-capacity ring buffer.
//! When the ring is full, the oldest entry spills to the storage backend
//! automatically via the spout. Explicit `flush()` drains everything.

use alloc::vec::Vec;
use core::hash::Hash;

use spill_ring::SpillRing;
use spout::Spout;

use crate::manager::traits::{CheckpointSerializer, Checkpointable};
use crate::storage::{CheckpointLoader, CheckpointMetadata, RecoverableStorage, SessionId};

pub use super::direct::DirectStorageError;
use super::{ColdTier, RecoverableColdTier};

/// SpillRing-backed cold tier.
///
/// Serialized checkpoints are pushed into a ring buffer of capacity `N`.
/// Overflow spills to the storage backend `S` automatically. `flush()`
/// drains the ring and flushes the storage.
///
/// Requires the `cold-buffer` feature.
///
/// # Type Parameters
/// - `CId` — Checkpoint ID type
/// - `S` — Storage backend implementing [`Spout`] (for writes) and
///   [`CheckpointLoader`] (for reads)
/// - `Ser` — Checkpoint serializer
/// - `N` — Ring buffer capacity (const generic)
pub struct RingCold<
    CId,
    S: Spout<(CId, Vec<u8>), Error = core::convert::Infallible>,
    Ser,
    const N: usize,
> {
    ring: SpillRing<(CId, Vec<u8>), N, S>,
    serializer: Ser,
}

impl<CId, S, Ser, const N: usize> RingCold<CId, S, Ser, N>
where
    CId: Copy,
    S: Spout<(CId, Vec<u8>), Error = core::convert::Infallible>,
{
    /// Create a new ring-buffered cold tier with a custom serializer.
    pub fn new(storage: S, serializer: Ser) -> Self {
        Self {
            ring: SpillRing::with_sink(storage),
            serializer,
        }
    }

    /// Borrow the underlying storage (through the ring's spout).
    pub fn storage(&self) -> &S {
        self.ring.sink_ref()
    }

    /// Mutably borrow the underlying storage.
    pub fn storage_mut(&mut self) -> &mut S {
        self.ring.sink_mut()
    }
}

#[cfg(feature = "bytecast")]
impl<CId, S, const N: usize> RingCold<CId, S, super::super::BytecastSerializer, N>
where
    CId: Copy,
    S: Spout<(CId, Vec<u8>), Error = core::convert::Infallible>,
{
    /// Create a new ring-buffered cold tier using `BytecastSerializer`.
    pub fn with_storage(storage: S) -> Self {
        Self::new(storage, super::super::BytecastSerializer)
    }
}

impl<T, S, Ser, const N: usize> ColdTier<T> for RingCold<T::Id, S, Ser, N>
where
    T: Checkpointable,
    S: Spout<(T::Id, Vec<u8>), Error = core::convert::Infallible> + CheckpointLoader<T::Id>,
    Ser: CheckpointSerializer<T>,
{
    type Error = DirectStorageError<Ser::Error>;

    fn store(&mut self, id: T::Id, checkpoint: &T) -> Result<(), Self::Error> {
        let bytes = self
            .serializer
            .serialize(checkpoint)
            .map_err(|source| DirectStorageError::Serializer { source })?;
        self.ring.push_mut((id, bytes));
        Ok(())
    }

    fn load(&self, id: T::Id) -> Result<T, Self::Error> {
        let bytes = self.ring.sink_ref().load(id)?;
        self.serializer
            .deserialize(&bytes)
            .map_err(|source| DirectStorageError::Serializer { source })
    }

    fn contains(&self, id: T::Id) -> bool {
        self.ring.sink_ref().contains(id)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.ring.flush();
        let _ = self.ring.sink_mut().flush();
        Ok(())
    }

    fn buffered_count(&self) -> usize {
        self.ring.len()
    }
}

impl<T, S, Ser, SId, const N: usize, const MAX_DEPS: usize> RecoverableColdTier<T, SId, MAX_DEPS>
    for RingCold<T::Id, S, Ser, N>
where
    T: Checkpointable,
    T::Id: Hash,
    S: Spout<(T::Id, Vec<u8>), Error = core::convert::Infallible>
        + RecoverableStorage<T::Id, SId, MAX_DEPS>,
    Ser: CheckpointSerializer<T>,
    SId: SessionId,
{
    type MetadataIter<'a>
        = RingMetadataIter<'a, T::Id, S, SId, MAX_DEPS>
    where
        Self: 'a,
        T::Id: 'a,
        SId: 'a;

    fn iter_metadata(&self) -> Self::MetadataIter<'_> {
        RingMetadataIter {
            inner: self.ring.sink_ref().iter_metadata(),
        }
    }

    fn get_metadata(&self, id: T::Id) -> Option<CheckpointMetadata<T::Id, SId, MAX_DEPS>> {
        self.ring.sink_ref().get_metadata(id)
    }
}

/// Iterator adapter for [`RingCold::iter_metadata`].
pub struct RingMetadataIter<'a, CId, S, SId, const MAX_DEPS: usize>
where
    CId: Copy + Eq + Hash + Default + core::fmt::Debug + 'a,
    S: RecoverableStorage<CId, SId, MAX_DEPS> + 'a,
    SId: SessionId + 'a,
{
    inner: S::MetadataIter<'a>,
}

impl<'a, CId, S, SId, const MAX_DEPS: usize> Iterator
    for RingMetadataIter<'a, CId, S, SId, MAX_DEPS>
where
    CId: Copy + Eq + Hash + Default + core::fmt::Debug,
    S: RecoverableStorage<CId, SId, MAX_DEPS>,
    SId: SessionId,
{
    type Item = (CId, CheckpointMetadata<CId, SId, MAX_DEPS>);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}
