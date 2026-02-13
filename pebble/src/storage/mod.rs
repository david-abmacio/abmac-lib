//! Storage abstraction for checkpoint persistence.

pub mod memory;

#[cfg(all(debug_assertions, feature = "std"))]
pub mod debug;

pub use memory::InMemoryStorage;

#[cfg(all(debug_assertions, feature = "std"))]
pub use debug::DebugFileStorage;

use alloc::vec::Vec;
use core::hash::Hash;
/// Session/correlation ID trait.
///
/// Built-in implementations: `u128`, `u64`, `()` (no tracking).
pub trait SessionId: Copy + Ord + Eq + Hash + Default + core::fmt::Debug {
    /// Extract timestamp if the ID encodes one.
    fn timestamp(&self) -> Option<u64> {
        None
    }
}

impl SessionId for u128 {}
impl SessionId for u64 {}
impl SessionId for () {}

pub use crate::errors::storage::{IntegrityError, IntegrityErrorKind, StorageError};

/// Load checkpoints from storage. Complement to `Spout` for writes.
pub trait CheckpointLoader<CId: Copy + Eq + Hash + core::fmt::Debug = u64> {
    /// Load serialized checkpoint data by ID.
    fn load(&self, state_id: CId) -> Result<Vec<u8>, StorageError>;

    /// Check if checkpoint exists.
    fn contains(&self, state_id: CId) -> bool;
}

/// Checkpoint metadata for self-describing recovery.
///
/// # Type Parameters
/// - `CId` - Checkpoint ID type
/// - `SId` - Session ID type
/// - `MAX_DEPS` - Maximum dependencies
pub struct CheckpointMetadata<
    CId: Copy + Eq + Hash + Default + core::fmt::Debug = u64,
    SId: SessionId = u128,
    const MAX_DEPS: usize = 8,
> {
    pub state_id: CId,
    dependencies: [CId; MAX_DEPS],
    pub dep_count: u8,
    pub creation_timestamp: u64,
    pub session_id: SId,
}

impl<CId: Copy + Eq + Hash + Default + core::fmt::Debug, SId: SessionId, const MAX_DEPS: usize>
    CheckpointMetadata<CId, SId, MAX_DEPS>
{
    /// Create metadata with no dependencies.
    pub fn new(state_id: CId, creation_timestamp: u64, session_id: SId) -> Self {
        Self {
            state_id,
            dependencies: [CId::default(); MAX_DEPS],
            dep_count: 0,
            creation_timestamp,
            session_id,
        }
    }

    /// Create metadata with dependencies.
    pub fn with_dependencies(
        state_id: CId,
        deps: &[CId],
        creation_timestamp: u64,
        session_id: SId,
    ) -> Result<Self, StorageError> {
        const { assert!(MAX_DEPS <= u8::MAX as usize, "MAX_DEPS must fit in u8") }

        if deps.len() > MAX_DEPS {
            return Err(StorageError::TooManyDependencies {
                max: MAX_DEPS,
                count: deps.len(),
            });
        }

        let mut dependencies = [CId::default(); MAX_DEPS];

        for (i, &dep) in deps.iter().enumerate() {
            dependencies[i] = dep;
        }

        Ok(Self {
            state_id,
            dependencies,
            dep_count: deps.len() as u8,
            creation_timestamp,
            session_id,
        })
    }

    /// Get dependencies as a slice.
    pub fn dependencies(&self) -> &[CId] {
        &self.dependencies[..self.dep_count as usize]
    }

    /// Maximum dependencies this metadata can hold.
    pub const fn max_dependencies() -> usize {
        MAX_DEPS
    }

    /// Get session timestamp if the ID encodes one.
    pub fn session_timestamp(&self) -> Option<u64> {
        self.session_id.timestamp()
    }
}

impl<CId: Copy + Eq + Hash + Default + core::fmt::Debug, SId: SessionId, const MAX_DEPS: usize>
    Clone for CheckpointMetadata<CId, SId, MAX_DEPS>
{
    fn clone(&self) -> Self {
        Self {
            state_id: self.state_id,
            dependencies: self.dependencies,
            dep_count: self.dep_count,
            creation_timestamp: self.creation_timestamp,
            session_id: self.session_id,
        }
    }
}

impl<CId: Copy + Eq + Hash + Default + core::fmt::Debug, SId: SessionId, const MAX_DEPS: usize>
    core::fmt::Debug for CheckpointMetadata<CId, SId, MAX_DEPS>
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CheckpointMetadata")
            .field("state_id", &self.state_id)
            .field("dependencies", &self.dependencies())
            .field("dep_count", &self.dep_count)
            .field("creation_timestamp", &self.creation_timestamp)
            .field("session_id", &self.session_id)
            .finish()
    }
}

impl<CId: Copy + Eq + Hash + Default + core::fmt::Debug, SId: SessionId, const MAX_DEPS: usize>
    PartialEq for CheckpointMetadata<CId, SId, MAX_DEPS>
{
    fn eq(&self, other: &Self) -> bool {
        self.state_id == other.state_id
            && self.dep_count == other.dep_count
            && self.dependencies() == other.dependencies()
            && self.creation_timestamp == other.creation_timestamp
            && self.session_id == other.session_id
    }
}

impl<CId: Copy + Eq + Hash + Default + core::fmt::Debug, SId: SessionId, const MAX_DEPS: usize> Eq
    for CheckpointMetadata<CId, SId, MAX_DEPS>
{
}

/// Recovery operation result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryResult {
    pub mode: RecoveryMode,
    pub checkpoints_loaded: usize,
    pub dag_nodes_rebuilt: usize,
    pub latest_state_id: Option<alloc::string::String>,
    pub integrity_errors: Vec<IntegrityError>,
}

/// How recovery completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryMode {
    /// No existing checkpoints found
    ColdStart,
    /// All checkpoints loaded successfully
    WarmRestart,
    /// Some checkpoints corrupted but recovered what we could
    PartialRecovery,
}

/// Storage with recovery support.
pub trait RecoverableStorage<
    CId: Copy + Eq + Hash + Default + core::fmt::Debug = u64,
    SId: SessionId = u128,
    const MAX_DEPS: usize = 8,
>: CheckpointLoader<CId>
{
    type MetadataIter<'a>: Iterator<Item = (CId, CheckpointMetadata<CId, SId, MAX_DEPS>)>
    where
        Self: 'a;

    /// Iterate all checkpoint metadata for discovery.
    fn iter_metadata(&self) -> Self::MetadataIter<'_>;

    /// Get metadata for a specific checkpoint.
    fn get_metadata(&self, state_id: CId) -> Option<CheckpointMetadata<CId, SId, MAX_DEPS>>;
}
