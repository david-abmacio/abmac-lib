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
//! use pebble::{Checkpoint, PebbleBuilder, InMemoryStorage, DirectStorage, NoWarm};
//! use spout::DropSpout;
//!
//! #[derive(Clone, Debug, Checkpoint)]
//! struct MyCheckpoint {
//!     #[checkpoint(id)]
//!     id: u64,
//!     data: Vec<u8>,
//! }
//!
//! let mut manager = PebbleBuilder::new()
//!     .cold(DirectStorage::new(InMemoryStorage::<u64, u128, 8>::new()))
//!     .warm(NoWarm)
//!     .log(DropSpout)
//!     .hot_capacity(16)
//!     .build::<MyCheckpoint>();
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

pub(crate) mod dag;
pub(crate) mod errors;
#[allow(dead_code)]
pub(crate) mod game;
pub mod manager;
pub(crate) mod storage;
pub(crate) mod strategy;

#[cfg(feature = "verdict")]
mod verdict_support;

#[cfg(test)]
mod tests;

pub use hashbrown::HashMap;

#[cfg(debug_assertions)]
pub use manager::DebugCold;
#[cfg(feature = "std")]
pub use manager::ParallelCold;
pub use manager::RingCold;
pub use manager::{
    BranchError, BranchId, BranchInfo, CapacityGuard, CheckpointRef, Checkpointable, ColdTier,
    DirectStorage, DirectStorageError, ErasedPebbleManagerError, HEAD, Manifest, ManifestEntry,
    Missing, NoWarm, PebbleBuilder, PebbleManager, PebbleManagerError, PebbleStats,
    RecoverableColdTier, Result, TheoreticalValidation, VerificationResult, WarmCache, WarmTier,
};
#[cfg(feature = "derive")]
pub use pebble_macros::Checkpoint;
#[cfg(all(debug_assertions, feature = "std"))]
pub use storage::DebugFileStorage;
pub use storage::{
    CheckpointLoader, CheckpointMetadata, InMemoryStorage, IntegrityError, IntegrityErrorKind,
    RecoverableStorage, RecoveryMode, RecoveryResult, SessionId, StorageError,
};

#[cfg(feature = "facet")]
pub use bytecast::BytecastFacet;
#[cfg(feature = "rkyv")]
pub use bytecast::BytecastRkyv;
#[cfg(feature = "serde")]
pub use bytecast::BytecastSerde;
pub use bytecast::{
    ByteSerializer, BytesError, FromBytes, FromBytesExt, ToBytes, ToBytesExt, ViewBytes,
};

pub use spout::Spout;
pub use strategy::{DAGPriorityMode, DAGStrategy, Strategy, TreeStrategy};

/// Prelude for convenient imports.
pub mod prelude {
    pub use crate::{
        BranchError, BranchId, BranchInfo, CapacityGuard, CheckpointLoader, CheckpointRef,
        Checkpointable, ColdTier, DirectStorage, ErasedPebbleManagerError, HEAD, HashMap,
        InMemoryStorage, Manifest, ManifestEntry, NoWarm, PebbleBuilder, PebbleManager,
        PebbleManagerError, RecoverableColdTier, Result, Spout, Strategy, WarmTier,
    };
}
