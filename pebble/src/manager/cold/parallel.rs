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
use super::{ColdMetadataIter, ColdTier, RecoverableColdTier};

/// Batch of already-serialized checkpoints to distribute across workers.
struct IoBatch<CId> {
    items: Vec<(CId, Vec<u8>, Vec<CId>)>,
    num_workers: usize,
}

/// Each worker iterates the batch, pushes items assigned to it
/// (round-robin by index) into its ring. Overflow spills to storage.
fn io_work<CId, const N: usize, S>(
    ring: &SpillRing<(CId, Vec<u8>, Vec<CId>), N, S>,
    worker_id: usize,
    batch: &IoBatch<CId>,
) where
    CId: Copy + Send,
    S: Spout<(CId, Vec<u8>, Vec<CId>), Error = core::convert::Infallible>,
{
    for (i, (id, bytes, deps)) in batch.items.iter().enumerate() {
        if i % batch.num_workers != worker_id {
            continue;
        }
        ring.push((*id, bytes.clone(), deps.clone()));
    }
}

type IoPool<CId, const N: usize, S> = spill_ring::WorkerPool<
    (CId, Vec<u8>, Vec<CId>),
    N,
    S,
    fn(&SpillRing<(CId, Vec<u8>, Vec<CId>), N, S>, usize, &IoBatch<CId>),
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
    S: Spout<(CId, Vec<u8>, Vec<CId>), Error = core::convert::Infallible> + Clone + Send + 'static,
{
    storage: S,
    pool: Option<IoPool<CId, N, S>>,
    num_workers: usize,
    /// Serialized items accumulated between `store()` calls.
    pending: Vec<(CId, Vec<u8>, Vec<CId>)>,
}

impl<CId, S, const N: usize> ParallelCold<CId, S, N>
where
    CId: Copy + Send + Sync + 'static,
    S: Spout<(CId, Vec<u8>, Vec<CId>), Error = core::convert::Infallible> + Clone + Send + 'static,
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
        MpscRing::<(CId, Vec<u8>, Vec<CId>), N, S>::pool_with_spout(num_workers, storage)
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
            // Drain any uncollected batches from a previous run before
            // dispatching new work. Without this, workers merge uncollected
            // batches back into their active rings, where items accumulate
            // until the next collect() — potentially past the ring's
            // capacity, triggering unnecessary spout overflow.
            match pool.collect(&mut self.storage) {
                Ok(_) => {}
                Err(e) => match e {},
            }
            pool.run(&batch);
        }
    }
}

impl<T, S, const N: usize> ColdTier<T> for ParallelCold<T::Id, S, N>
where
    T: Checkpointable + bytecast::ToBytes + bytecast::FromBytes,
    T::Id: Send + Sync + 'static,
    S: Spout<(T::Id, Vec<u8>, Vec<T::Id>), Error = core::convert::Infallible>
        + CheckpointLoader<T::Id>
        + CheckpointRemover<T::Id>
        + Clone
        + Send
        + 'static,
{
    type Error = DirectStorageError;

    fn store(&mut self, id: T::Id, checkpoint: &T, deps: &[T::Id]) -> Result<(), Self::Error> {
        // Serialize eagerly on the caller's thread.
        let bytes = ByteSerializer
            .serialize(checkpoint)
            .map_err(|source| DirectStorageError::Serializer { source })?;

        self.pending.push((id, bytes, deps.to_vec()));
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

        // Drain handoff slots directly to storage. The pool stays alive —
        // no thread teardown/respawn, no ring reallocation.
        if let Some(pool) = self.pool.as_mut() {
            match pool.collect(&mut self.storage) {
                Ok(_) => {}
                Err(e) => match e {},
            }
        }
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
    S: Spout<(T::Id, Vec<u8>, Vec<T::Id>), Error = core::convert::Infallible>
        + RecoverableStorage<T::Id, SId, MAX_DEPS>
        + CheckpointRemover<T::Id>
        + Clone
        + Send
        + 'static,
    SId: SessionId,
{
    type MetadataIter<'a>
        = ColdMetadataIter<'a, T::Id, S, SId, MAX_DEPS>
    where
        Self: 'a,
        T::Id: 'a,
        SId: 'a;

    fn iter_metadata(&self) -> Self::MetadataIter<'_> {
        ColdMetadataIter {
            inner: self.storage.iter_metadata(),
        }
    }

    fn get_metadata(&self, id: T::Id) -> Option<CheckpointMetadata<T::Id, SId, MAX_DEPS>> {
        self.storage.get_metadata(id)
    }
}

// ParallelCold buffers serialized items in `pending` between store() calls.
// PebbleManager::Drop calls flush() which drains pending, but standalone
// usage must flush explicitly before dropping — matching the Spout
// ecosystem convention (BatchSpout, ReduceSpout, etc.).
impl<CId, S, const N: usize> Drop for ParallelCold<CId, S, N>
where
    CId: Copy + Send + Sync + 'static,
    S: Spout<(CId, Vec<u8>, Vec<CId>), Error = core::convert::Infallible> + Clone + Send + 'static,
{
    fn drop(&mut self) {
        debug_assert!(
            self.pending.is_empty(),
            "ParallelCold dropped with {} unflushed items — call flush() before dropping",
            self.pending.len(),
        );
    }
}
