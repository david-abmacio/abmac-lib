//! WorkerPool-backed cold tier for parallel I/O.
//!
//! Checkpoints are serialized eagerly in `store()`, then batched and
//! distributed across worker threads for parallel I/O. Each worker
//! owns a SpillRing; overflow spills to shared storage automatically.
//! Requires the `std` feature.

extern crate alloc;
extern crate std;

use alloc::vec::Vec;
use core::hash::Hash;

use spill_ring::{MpscRing, SpillRing};
use spout::Spout;

use crate::manager::traits::Checkpointable;
use crate::storage::{
    CheckpointLoader, CheckpointMetadata, CheckpointRemover, RecoverableStorage, SessionId,
};
use bytecast::ByteSerializer;

pub use super::direct::DirectStorageError;
use super::{ColdTier, RecoverableColdTier};

/// Batch of already-serialized checkpoints to distribute across workers.
struct IoBatch<CId> {
    items: Vec<(CId, Vec<u8>)>,
    num_workers: usize,
}

/// Each worker iterates the batch, pushes items assigned to it
/// (round-robin by index) into its ring. Overflow spills to storage.
fn io_work<CId, const N: usize, S>(
    ring: &SpillRing<(CId, Vec<u8>), N, S>,
    worker_id: usize,
    batch: &IoBatch<CId>,
) where
    CId: Copy + Send,
    S: Spout<(CId, Vec<u8>), Error = core::convert::Infallible>,
{
    for (i, (id, bytes)) in batch.items.iter().enumerate() {
        if i % batch.num_workers != worker_id {
            continue;
        }
        ring.push((*id, bytes.clone()));
    }
}

type IoPool<CId, const N: usize, S> = spill_ring::WorkerPool<
    (CId, Vec<u8>),
    N,
    S,
    fn(&SpillRing<(CId, Vec<u8>), N, S>, usize, &IoBatch<CId>),
    IoBatch<CId>,
>;

/// WorkerPool-backed cold tier for parallel I/O.
///
/// Checkpoints are serialized eagerly in [`store()`](ColdTier::store),
/// then batched and distributed across `num_workers` threads. Each
/// worker owns a SpillRing of capacity `N`; overflow spills to shared
/// storage via the spout. [`flush()`](ColdTier::flush) joins all
/// workers, drains remaining items to storage, then recreates the pool.
///
/// Requires the `std` feature.
///
/// # Type Parameters
/// - `CId` — Checkpoint ID type
/// - `S` — Storage backend (must be `Clone + Send + 'static`)
/// - `N` — Per-worker ring buffer capacity (const generic)
pub struct ParallelCold<CId, S, const N: usize>
where
    CId: Copy + Send + Sync + 'static,
    S: Spout<(CId, Vec<u8>), Error = core::convert::Infallible> + Clone + Send + 'static,
{
    storage: S,
    pool: Option<IoPool<CId, N, S>>,
    num_workers: usize,
    /// Serialized items accumulated between `store()` calls.
    pending: Vec<(CId, Vec<u8>)>,
}

impl<CId, S, const N: usize> ParallelCold<CId, S, N>
where
    CId: Copy + Send + Sync + 'static,
    S: Spout<(CId, Vec<u8>), Error = core::convert::Infallible> + Clone + Send + 'static,
{
    /// Create a new parallel cold tier.
    pub fn new(storage: S, num_workers: usize) -> Self {
        let pool = Self::create_pool(num_workers, storage.clone());
        Self {
            storage,
            pool: Some(pool),
            num_workers,
            pending: Vec::new(),
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

    /// Number of worker threads.
    pub fn num_workers(&self) -> usize {
        self.num_workers
    }

    fn create_pool(num_workers: usize, storage: S) -> IoPool<CId, N, S> {
        MpscRing::<(CId, Vec<u8>), N, S>::pool_with_sink(num_workers, storage)
            .spawn(io_work::<CId, N, S>)
    }

    /// Distribute pending serialized items across worker threads.
    fn run_batch(&mut self) {
        if self.pending.is_empty() {
            return;
        }

        let batch = IoBatch {
            items: core::mem::take(&mut self.pending),
            num_workers: self.num_workers,
        };

        if let Some(pool) = self.pool.as_mut() {
            pool.run(&batch);
        }
    }
}

impl<T, S, const N: usize> ColdTier<T> for ParallelCold<T::Id, S, N>
where
    T: Checkpointable + bytecast::ToBytes + bytecast::FromBytes,
    T::Id: Send + Sync + 'static,
    S: Spout<(T::Id, Vec<u8>), Error = core::convert::Infallible>
        + CheckpointLoader<T::Id>
        + CheckpointRemover<T::Id>
        + Clone
        + Send
        + 'static,
{
    type Error = DirectStorageError;

    fn store(&mut self, id: T::Id, checkpoint: &T) -> Result<(), Self::Error> {
        // Serialize eagerly on the caller's thread.
        let bytes = ByteSerializer
            .serialize(checkpoint)
            .map_err(|source| DirectStorageError::Serializer { source })?;

        self.pending.push((id, bytes));
        // Distribute to workers when we have enough items to keep them busy.
        if self.pending.len() >= self.num_workers {
            self.run_batch();
        }
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
        // Flush any pending items through the pool.
        self.run_batch();

        // Take the pool, drain worker rings to storage, recreate.
        if let Some(pool) = self.pool.take() {
            let mut consumer = pool.into_consumer();
            consumer.drain(&mut self.storage);
        }
        self.pool = Some(Self::create_pool(self.num_workers, self.storage.clone()));
        Ok(())
    }

    fn remove(&mut self, id: T::Id) -> Result<bool, Self::Error> {
        Ok(self.storage.remove(id))
    }

    fn buffered_count(&self) -> usize {
        self.pending.len()
    }
}

impl<T, S, SId, const N: usize, const MAX_DEPS: usize> RecoverableColdTier<T, SId, MAX_DEPS>
    for ParallelCold<T::Id, S, N>
where
    T: Checkpointable + bytecast::ToBytes + bytecast::FromBytes,
    T::Id: Hash + Send + Sync + 'static,
    S: Spout<(T::Id, Vec<u8>), Error = core::convert::Infallible>
        + RecoverableStorage<T::Id, SId, MAX_DEPS>
        + CheckpointRemover<T::Id>
        + Clone
        + Send
        + 'static,
    SId: SessionId,
{
    type MetadataIter<'a>
        = ParallelMetadataIter<'a, T::Id, S, SId, MAX_DEPS>
    where
        Self: 'a,
        T::Id: 'a,
        SId: 'a;

    fn iter_metadata(&self) -> Self::MetadataIter<'_> {
        ParallelMetadataIter {
            inner: self.storage.iter_metadata(),
        }
    }

    fn get_metadata(&self, id: T::Id) -> Option<CheckpointMetadata<T::Id, SId, MAX_DEPS>> {
        self.storage.get_metadata(id)
    }
}

/// Iterator adapter for [`ParallelCold::iter_metadata`].
pub struct ParallelMetadataIter<'a, CId, S, SId, const MAX_DEPS: usize>
where
    CId: Copy + Eq + Hash + Default + core::fmt::Debug + 'a,
    S: RecoverableStorage<CId, SId, MAX_DEPS> + 'a,
    SId: SessionId + 'a,
{
    inner: S::MetadataIter<'a>,
}

impl<'a, CId, S, SId, const MAX_DEPS: usize> Iterator
    for ParallelMetadataIter<'a, CId, S, SId, MAX_DEPS>
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

// ParallelCold buffers serialized items in `pending` between store() calls.
// PebbleManager::Drop calls flush() which drains pending, but standalone
// usage must flush explicitly before dropping — matching the Spout
// ecosystem convention (BatchSpout, ReduceSpout, etc.).
impl<CId, S, const N: usize> Drop for ParallelCold<CId, S, N>
where
    CId: Copy + Send + Sync + 'static,
    S: Spout<(CId, Vec<u8>), Error = core::convert::Infallible> + Clone + Send + 'static,
{
    fn drop(&mut self) {
        debug_assert!(
            self.pending.is_empty(),
            "ParallelCold dropped with {} unflushed items — call flush() before dropping",
            self.pending.len(),
        );
    }
}
