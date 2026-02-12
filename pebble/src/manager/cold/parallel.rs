//! WorkerPool-backed cold tier for parallel serialization.
//!
//! Checkpoints are batched and serialized across multiple threads.
//! Each worker owns a SpillRing; overflow spills to shared storage
//! automatically. Requires the `cold-buffer-std` feature.

extern crate alloc;
extern crate std;

use alloc::vec::Vec;
use core::hash::Hash;
use std::sync::{Arc, Mutex};

use spill_ring::{MpscRing, SpillRing};
use spout::Spout;

use crate::manager::traits::{CheckpointSerializer, Checkpointable};
use crate::storage::{CheckpointLoader, CheckpointMetadata, RecoverableStorage, SessionId};

pub use super::direct::DirectStorageError;
use super::{ColdTier, RecoverableColdTier};

/// Shared error collection across worker threads.
type SharedErrors<Id, E> = Arc<Mutex<Vec<(Id, E)>>>;

/// Batch of checkpoints to serialize in parallel.
///
/// Passed as the `A` argument to `WorkerPool::run()`. Workers partition
/// items by index and serialize their share. Errors are collected into
/// the shared `errors` vec.
struct SerializeBatch<T: Checkpointable, Ser: CheckpointSerializer<T>>
where
    T::Id: Sync,
{
    items: Vec<(T::Id, T)>,
    serializer: Ser,
    num_workers: usize,
    errors: SharedErrors<T::Id, Ser::Error>,
}

// SAFETY: SerializeBatch is Sync when T, T::Id, Ser, and Ser::Error are Sync.
// The Arc<Mutex<Vec>> is always Sync. Items are read-only during worker execution.
// The Sync bound on T::Id is enforced by the where clause.

/// Each worker iterates the batch, serializes items assigned to it
/// (round-robin by index), and pushes `(id, bytes)` into its ring.
/// Serialization errors are collected into the batch's error vec.
fn serialize_work<T, Ser, const N: usize, S>(
    ring: &SpillRing<(T::Id, Vec<u8>), N, S>,
    worker_id: usize,
    batch: &SerializeBatch<T, Ser>,
) where
    T: Checkpointable,
    T::Id: Sync,
    Ser: CheckpointSerializer<T>,
    S: Spout<(T::Id, Vec<u8>)>,
{
    for (i, (id, checkpoint)) in batch.items.iter().enumerate() {
        if i % batch.num_workers != worker_id {
            continue;
        }
        match batch.serializer.serialize(checkpoint) {
            Ok(bytes) => ring.push((*id, bytes)),
            Err(e) => {
                if let Ok(mut errs) = batch.errors.lock() {
                    errs.push((*id, e));
                }
            }
        }
    }
}

type SerializePool<T, Ser, const N: usize, S> = spill_ring::WorkerPool<
    (<T as Checkpointable>::Id, Vec<u8>),
    N,
    S,
    fn(&SpillRing<(<T as Checkpointable>::Id, Vec<u8>), N, S>, usize, &SerializeBatch<T, Ser>),
    SerializeBatch<T, Ser>,
>;

/// WorkerPool-backed cold tier for parallel serialization.
///
/// Checkpoints are collected into a batch, then serialized across
/// `num_workers` threads. Each worker owns a SpillRing of capacity `N`;
/// overflow spills to shared storage via the spout. `flush()` joins
/// all workers, drains remaining items to storage, then recreates the
/// pool.
///
/// Requires the `cold-buffer-std` feature.
///
/// # Type Parameters
/// - `T` — Checkpointable type
/// - `S` — Storage backend (must be `Clone + Send + 'static`)
/// - `Ser` — Checkpoint serializer (must be `Clone + Sync + 'static`)
/// - `N` — Per-worker ring buffer capacity (const generic)
pub struct ParallelCold<T, S, Ser, const N: usize>
where
    T: Checkpointable + Send + Sync + 'static,
    T::Id: Send + Sync + 'static,
    S: Spout<(T::Id, Vec<u8>)> + Clone + Send + 'static,
    Ser: CheckpointSerializer<T> + Clone + Sync + 'static,
    Ser::Error: Send + 'static,
{
    storage: S,
    serializer: Ser,
    pool: Option<SerializePool<T, Ser, N, S>>,
    num_workers: usize,
    /// Pending items accumulated between `store()` calls.
    pending: Vec<(T::Id, T)>,
    /// Shared error collection across batches.
    errors: SharedErrors<T::Id, Ser::Error>,
}

impl<T, S, Ser, const N: usize> ParallelCold<T, S, Ser, N>
where
    T: Checkpointable + Send + Sync + 'static,
    T::Id: Send + Sync + 'static,
    S: Spout<(T::Id, Vec<u8>)> + CheckpointLoader<T::Id> + Clone + Send + 'static,
    Ser: CheckpointSerializer<T> + Clone + Send + Sync + 'static,
    Ser::Error: Send + 'static,
{
    /// Create a new parallel cold tier with a custom serializer.
    pub fn new(storage: S, serializer: Ser, num_workers: usize) -> Self {
        let pool = Self::create_pool(num_workers, storage.clone());
        Self {
            storage,
            serializer,
            pool: Some(pool),
            num_workers,
            pending: Vec::new(),
            errors: Arc::new(Mutex::new(Vec::new())),
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

    fn create_pool(num_workers: usize, storage: S) -> SerializePool<T, Ser, N, S> {
        MpscRing::<(T::Id, Vec<u8>), N, S>::pool_with_sink(num_workers, storage)
            .spawn(serialize_work::<T, Ser, N, S>)
    }

    /// Run a batch of pending items through the worker pool.
    fn run_batch(&mut self) {
        if self.pending.is_empty() {
            return;
        }

        let batch = SerializeBatch {
            items: core::mem::take(&mut self.pending),
            serializer: self.serializer.clone(),
            num_workers: self.num_workers,
            errors: Arc::clone(&self.errors),
        };

        if let Some(pool) = self.pool.as_mut() {
            pool.run(&batch);
        }
    }

    /// Surface the first serialization error from the worker pool, if any.
    fn check_errors(&self) -> Result<(), DirectStorageError<Ser::Error>> {
        match self.errors.lock() {
            Ok(mut errs) => {
                if let Some((_id, err)) = errs.drain(..).next() {
                    Err(DirectStorageError::Serializer(err))
                } else {
                    Ok(())
                }
            }
            Err(poisoned) => {
                // Mutex poisoned — a worker panicked. Drain what we can.
                let mut errs = poisoned.into_inner();
                if let Some((_id, err)) = errs.drain(..).next() {
                    Err(DirectStorageError::Serializer(err))
                } else {
                    Ok(())
                }
            }
        }
    }
}

#[cfg(feature = "bytecast")]
impl<T, S, const N: usize> ParallelCold<T, S, crate::manager::BytecastSerializer, N>
where
    T: Checkpointable + bytecast::ToBytes + bytecast::FromBytes + Send + Sync + 'static,
    T::Id: Send + Sync + 'static,
    S: Spout<(T::Id, Vec<u8>)> + CheckpointLoader<T::Id> + Clone + Send + 'static,
{
    /// Create a new parallel cold tier using `BytecastSerializer`.
    pub fn with_storage(storage: S, num_workers: usize) -> Self {
        Self::new(storage, crate::manager::BytecastSerializer, num_workers)
    }
}

impl<T, S, Ser, const N: usize> ColdTier<T> for ParallelCold<T, S, Ser, N>
where
    T: Checkpointable + Send + Sync + 'static,
    T::Id: Send + Sync + 'static,
    S: Spout<(T::Id, Vec<u8>)> + CheckpointLoader<T::Id> + Clone + Send + 'static,
    Ser: CheckpointSerializer<T> + Clone + Send + Sync + 'static,
    Ser::Error: Send + 'static,
{
    type Error = DirectStorageError<Ser::Error>;

    fn store(&mut self, id: T::Id, checkpoint: &T) -> Result<(), Self::Error> {
        // Check for errors from previous batches before accepting more work.
        self.check_errors()?;

        self.pending.push((id, checkpoint.clone()));
        // Run batch when we have enough items to keep workers busy.
        if self.pending.len() >= self.num_workers {
            self.run_batch();
            self.check_errors()?;
        }
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
        // Flush any pending items through the pool.
        self.run_batch();
        self.check_errors()?;

        // Take the pool, drain worker rings to storage, recreate.
        if let Some(pool) = self.pool.take() {
            let mut consumer = pool.into_consumer();
            consumer.drain(&mut self.storage);
        }
        self.pool = Some(Self::create_pool(self.num_workers, self.storage.clone()));
        Ok(())
    }

    fn buffered_count(&self) -> usize {
        self.pending.len()
    }
}

impl<T, S, Ser, SId, const N: usize, const MAX_DEPS: usize> RecoverableColdTier<T, SId, MAX_DEPS>
    for ParallelCold<T, S, Ser, N>
where
    T: Checkpointable + Send + Sync + 'static,
    T::Id: Hash + Send + Sync + 'static,
    S: Spout<(T::Id, Vec<u8>)> + RecoverableStorage<T::Id, SId, MAX_DEPS> + Clone + Send + 'static,
    Ser: CheckpointSerializer<T> + Clone + Send + Sync + 'static,
    Ser::Error: Send + 'static,
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
    CId: Copy + Eq + Hash + core::fmt::Debug + 'a,
    S: RecoverableStorage<CId, SId, MAX_DEPS> + 'a,
    SId: SessionId + 'a,
{
    inner: S::MetadataIter<'a>,
}

impl<'a, CId, S, SId, const MAX_DEPS: usize> Iterator
    for ParallelMetadataIter<'a, CId, S, SId, MAX_DEPS>
where
    CId: Copy + Eq + Hash + core::fmt::Debug,
    S: RecoverableStorage<CId, SId, MAX_DEPS>,
    SId: SessionId,
{
    type Item = (CId, CheckpointMetadata<CId, SId, MAX_DEPS>);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}
