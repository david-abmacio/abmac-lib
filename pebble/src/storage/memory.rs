//! In-memory storage implementation for testing and development.

use alloc::vec::Vec;
use core::hash::Hash;
use hashbrown::HashMap;
use spout::Spout;

use super::{
    CheckpointLoader, CheckpointMetadata, CheckpointRemover, RecoverableStorage, SessionId,
    StorageError,
};

/// In-memory storage for testing. Not thread-safe.
#[derive(Debug, Clone)]
pub struct InMemoryStorage<
    CId: Copy + Eq + Hash + Default + core::fmt::Debug = u64,
    SId: SessionId = u128,
    const MAX_DEPS: usize = 8,
> {
    data: HashMap<CId, Vec<u8>>,
    metadata: HashMap<CId, CheckpointMetadata<CId, SId, MAX_DEPS>>,
    next_timestamp: u64,
}

impl<CId: Copy + Eq + Hash + Default + core::fmt::Debug, SId: SessionId, const MAX_DEPS: usize>
    Default for InMemoryStorage<CId, SId, MAX_DEPS>
{
    fn default() -> Self {
        Self {
            data: HashMap::new(),
            metadata: HashMap::new(),
            next_timestamp: 1,
        }
    }
}

impl<CId: Copy + Eq + Hash + Default + core::fmt::Debug, SId: SessionId, const MAX_DEPS: usize>
    InMemoryStorage<CId, SId, MAX_DEPS>
{
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn store_with_metadata(
        &mut self,
        state_id: CId,
        data: Vec<u8>,
        metadata: CheckpointMetadata<CId, SId, MAX_DEPS>,
    ) {
        self.data.insert(state_id, data);
        self.metadata.insert(state_id, metadata);
    }

    pub fn remove(&mut self, state_id: CId) -> bool {
        // Both removes execute before combining results.
        let data = self.data.remove(&state_id).is_some();
        let meta = self.metadata.remove(&state_id).is_some();
        data | meta
    }
}

impl<CId: Copy + Eq + Hash + Default + core::fmt::Debug, SId: SessionId, const MAX_DEPS: usize>
    Spout<(CId, Vec<u8>, Vec<CId>)> for InMemoryStorage<CId, SId, MAX_DEPS>
{
    type Error = core::convert::Infallible;

    fn send(&mut self, item: (CId, Vec<u8>, Vec<CId>)) -> Result<(), Self::Error> {
        let (state_id, data, deps) = item;
        let ts = self.next_timestamp;
        self.next_timestamp = ts.wrapping_add(1);
        // with_dependencies can only fail if deps.len() > MAX_DEPS.
        // ColdTier callers are bounded by the same MAX_DEPS, so this
        // is safe to unwrap. Use expect for a clear message if it
        // ever fires in a misconfigured setup.
        let metadata = CheckpointMetadata::with_dependencies(state_id, &deps, ts, SId::default())
            .expect("dependency count exceeds MAX_DEPS");
        self.store_with_metadata(state_id, data, metadata);
        Ok(())
    }
}

impl<CId: Copy + Eq + Hash + Default + core::fmt::Debug, SId: SessionId, const MAX_DEPS: usize>
    CheckpointLoader<CId> for InMemoryStorage<CId, SId, MAX_DEPS>
{
    fn load(&self, state_id: CId) -> Result<Vec<u8>, StorageError> {
        self.data
            .get(&state_id)
            .cloned()
            .ok_or(StorageError::NotFound)
    }

    fn contains(&self, state_id: CId) -> bool {
        self.data.contains_key(&state_id)
    }
}

impl<CId: Copy + Eq + Hash + Default + core::fmt::Debug, SId: SessionId, const MAX_DEPS: usize>
    CheckpointRemover<CId> for InMemoryStorage<CId, SId, MAX_DEPS>
{
    fn remove(&mut self, state_id: CId) -> bool {
        self.remove(state_id)
    }
}

impl<CId: Copy + Eq + Hash + Default + core::fmt::Debug, SId: SessionId, const MAX_DEPS: usize>
    RecoverableStorage<CId, SId, MAX_DEPS> for InMemoryStorage<CId, SId, MAX_DEPS>
{
    type MetadataIter<'a>
        = InMemoryMetadataIter<'a, CId, SId, MAX_DEPS>
    where
        CId: 'a,
        SId: 'a;

    fn iter_metadata(&self) -> Self::MetadataIter<'_> {
        InMemoryMetadataIter {
            inner: self.metadata.iter(),
        }
    }

    fn get_metadata(&self, state_id: CId) -> Option<CheckpointMetadata<CId, SId, MAX_DEPS>> {
        self.metadata.get(&state_id).cloned()
    }
}

/// Iterator over checkpoint metadata.
pub struct InMemoryMetadataIter<
    'a,
    CId: Copy + Eq + Hash + Default + core::fmt::Debug = u64,
    SId: SessionId = u128,
    const MAX_DEPS: usize = 8,
> {
    inner: hashbrown::hash_map::Iter<'a, CId, CheckpointMetadata<CId, SId, MAX_DEPS>>,
}

impl<'a, CId: Copy + Eq + Hash + Default + core::fmt::Debug, SId: SessionId, const MAX_DEPS: usize>
    Iterator for InMemoryMetadataIter<'a, CId, SId, MAX_DEPS>
{
    type Item = (CId, CheckpointMetadata<CId, SId, MAX_DEPS>);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(&id, meta)| (id, meta.clone()))
    }
}
