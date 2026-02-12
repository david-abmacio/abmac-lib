//! In-memory storage implementation for testing and development.

use alloc::vec::Vec;
use core::hash::Hash;
use hashbrown::HashMap;
use spout::Spout;

use super::{CheckpointLoader, CheckpointMetadata, RecoverableStorage, SessionId, StorageError};

/// In-memory storage for testing. Not thread-safe.
#[derive(Debug)]
pub struct InMemoryStorage<
    CId: Copy + Eq + Hash + core::fmt::Debug = u64,
    SId: SessionId = u128,
    const MAX_DEPS: usize = 8,
> {
    data: HashMap<CId, Vec<u8>>,
    metadata: HashMap<CId, CheckpointMetadata<CId, SId, MAX_DEPS>>,
    next_timestamp: u64,
}

impl<CId: Copy + Eq + Hash + core::fmt::Debug, SId: SessionId, const MAX_DEPS: usize> Default
    for InMemoryStorage<CId, SId, MAX_DEPS>
{
    fn default() -> Self {
        Self {
            data: HashMap::new(),
            metadata: HashMap::new(),
            next_timestamp: 1,
        }
    }
}

impl<CId: Copy + Eq + Hash + core::fmt::Debug, SId: SessionId, const MAX_DEPS: usize>
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
        // Bitwise OR: both removes must execute (|| would short-circuit).
        let data = self.data.remove(&state_id).is_some();
        let meta = self.metadata.remove(&state_id).is_some();
        data || meta
    }
}

impl<CId: Copy + Eq + Hash + core::fmt::Debug, SId: SessionId, const MAX_DEPS: usize>
    Spout<(CId, Vec<u8>)> for InMemoryStorage<CId, SId, MAX_DEPS>
{
    fn send(&mut self, item: (CId, Vec<u8>)) {
        let (state_id, data) = item;
        let ts = self.next_timestamp;
        self.next_timestamp = ts.wrapping_add(1);
        let metadata = CheckpointMetadata::new(state_id, ts, SId::default());
        self.store_with_metadata(state_id, data, metadata);
    }
}

impl<CId: Copy + Eq + Hash + core::fmt::Debug, SId: SessionId, const MAX_DEPS: usize>
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

impl<CId: Copy + Eq + Hash + core::fmt::Debug, SId: SessionId, const MAX_DEPS: usize>
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
    CId: Copy + Eq + Hash + core::fmt::Debug = u64,
    SId: SessionId = u128,
    const MAX_DEPS: usize = 8,
> {
    inner: hashbrown::hash_map::Iter<'a, CId, CheckpointMetadata<CId, SId, MAX_DEPS>>,
}

impl<'a, CId: Copy + Eq + Hash + core::fmt::Debug, SId: SessionId, const MAX_DEPS: usize> Iterator
    for InMemoryMetadataIter<'a, CId, SId, MAX_DEPS>
{
    type Item = (CId, CheckpointMetadata<CId, SId, MAX_DEPS>);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(&id, meta)| (id, meta.clone()))
    }
}

/// IEEE 802.3 CRC32 polynomial (reversed bit order).
const CRC32_POLYNOMIAL: u32 = 0xEDB8_8320;
const CRC32_INIT: u32 = 0xFFFF_FFFFu32;
const BITS_PER_BYTE: usize = 8;

const fn crc32_entry(byte: u8) -> u32 {
    let mut crc = byte as u32;
    let mut i = 0;
    while i < BITS_PER_BYTE {
        crc = if crc & 1 != 0 {
            (crc >> 1) ^ CRC32_POLYNOMIAL
        } else {
            crc >> 1
        };
        i += 1;
    }
    crc
}

const CRC32_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        table[i] = crc32_entry(i as u8);
        i += 1;
    }
    table
};

/// CRC32 checksum (IEEE polynomial).
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = CRC32_INIT;
    for &byte in data {
        let index = ((crc ^ byte as u32) & 0xFF) as usize;
        crc = (crc >> 8) ^ CRC32_TABLE[index];
    }
    !crc
}
