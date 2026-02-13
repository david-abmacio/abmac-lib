//! O(sqrt(T)) checkpoint management using the Williams pebble game algorithm.
//!
//! # Overview
//!
//! Pebble provides efficient checkpoint storage for event-sourced systems with
//! bounded memory usage and fast replay capabilities.
//!
//! - **O(sqrt(T)) space** - Store T events with sqrt(T) checkpoints in memory
//! - **O(sqrt(T)) replay** - Rebuild any state with at most sqrt(T) operations
//! - **no_std compatible** - Works in embedded environments (requires `alloc`)
//! - **Pluggable storage** - Abstract over flash, disk, or custom backends
//!
//! # Quick Start
//!
//! ```
//! use pebble::{PebbleManagerBuilder, InMemoryStorage, Checkpointable, CheckpointSerializer, DirectStorage, Manifest, NoWarm};
//! use spout::DropSpout;
//!
//! #[derive(Clone)]
//! struct MyCheckpoint { id: u64, data: Vec<u8> }
//!
//! impl Checkpointable for MyCheckpoint {
//!     type Id = u64;
//!     type RebuildError = ();
//!     fn checkpoint_id(&self) -> u64 { self.id }
//!     fn compute_from_dependencies(
//!         base: Self, _deps: &hashbrown::HashMap<Self::Id, &Self>,
//!     ) -> Result<Self, Self::RebuildError> { Ok(base) }
//! }
//!
//! struct MySerializer;
//! impl CheckpointSerializer<MyCheckpoint> for MySerializer {
//!     type Error = &'static str;
//!     fn serialize(&self, _cp: &MyCheckpoint) -> Result<Vec<u8>, &'static str> { Ok(vec![]) }
//!     fn deserialize(&self, _bytes: &[u8]) -> Result<MyCheckpoint, &'static str> {
//!         Ok(MyCheckpoint { id: 0, data: vec![] })
//!     }
//! }
//!
//! let cold = DirectStorage::new(InMemoryStorage::<u64, u128, 8>::new(), MySerializer);
//! let manifest = Manifest::new(DropSpout);
//! let mut manager = PebbleManagerBuilder::new()
//!     .cold(cold)
//!     .warm(NoWarm)
//!     .hot_capacity(16)
//!     .build::<MyCheckpoint, _>(manifest)
//!     .unwrap();
//!
//! manager.add(MyCheckpoint { id: 1, data: vec![1, 2, 3] }, &[]).unwrap();
//! ```
//!
//! # Thread Safety
//!
//! Core types are not internally synchronized. Wrap in `Mutex` or `RwLock` for
//! concurrent access.

#![no_std]

extern crate alloc;

pub mod dag;
pub mod errors;
pub mod game;
pub mod manager;
pub mod storage;
pub mod strategy;

#[cfg(feature = "verdict")]
mod verdict_support;

#[cfg(test)]
mod tests;

pub use hashbrown::HashMap;

pub use dag::{ComputationDAG, DAGError, DAGNode, DAGStats};
pub use game::{PebbleColor, PebbleError, PebbleGame, PebbleOperation, PebbleRules};
#[cfg(feature = "bytecast")]
pub use manager::BytecastSerializer;
#[cfg(feature = "cold-buffer-std")]
pub use manager::ParallelCold;
#[cfg(feature = "cold-buffer")]
pub use manager::RingCold;
pub use manager::{
    BranchError, BranchId, BranchInfo, BuilderError, CapacityGuard, CheckpointRef,
    CheckpointSerializer, Checkpointable, ColdTier, DirectStorage, DirectStorageError,
    ErasedPebbleManagerError, HEAD, Manifest, ManifestEntry, NoWarm, PebbleManager,
    PebbleManagerBuilder, PebbleManagerError, PebbleStats, RecoverableColdTier, Result,
    TheoreticalValidation, VerificationResult, WarmCache, WarmTier,
};
pub use storage::{
    CheckpointLoader, CheckpointMetadata, InMemoryStorage, IntegrityError, IntegrityErrorKind,
    RecoverableStorage, RecoveryMode, RecoveryResult, SessionId, StorageError, crc32,
};

pub use spout::Spout;
pub use strategy::{DAGPriorityMode, DAGStrategy, Strategy, TreeStrategy};

/// Integer square root (floor).
#[must_use]
#[inline]
pub const fn isqrt(n: u64) -> u64 {
    n.isqrt()
}

/// Optimal checkpoint interval for T events (returns sqrt(T), minimum 1).
#[must_use]
#[inline]
pub const fn checkpoint_interval(expected_events: u64) -> u64 {
    let sqrt = isqrt(expected_events);
    if sqrt == 0 { 1 } else { sqrt }
}

/// Prelude for convenient imports.
pub mod prelude {
    pub use crate::{
        BranchError, BranchId, BranchInfo, BuilderError, CapacityGuard, CheckpointLoader,
        CheckpointRef, CheckpointSerializer, Checkpointable, ColdTier, DirectStorage,
        ErasedPebbleManagerError, HEAD, HashMap, InMemoryStorage, Manifest, ManifestEntry, NoWarm,
        PebbleManager, PebbleManagerBuilder, PebbleManagerError, RecoverableColdTier, Result,
        Spout, Strategy, WarmTier, checkpoint_interval, isqrt,
    };
}
