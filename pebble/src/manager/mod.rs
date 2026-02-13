//! Checkpoint management using Williams' pebble game algorithm.
//!
//! This module provides `PebbleManager`, a high-level interface for managing
//! checkpoints with O(√T) space complexity and near-optimal I/O operations.

mod branch;
mod branching;
mod builder;
pub mod cold;
mod error;
mod manifest;
mod pebble_manager;
mod rebuild;
mod recovery;
mod safety;
mod serializers;
mod stats;
mod traits;
mod verify;
pub mod warm;

pub use branch::{BranchError, BranchId, BranchInfo, HEAD};
pub use builder::PebbleManagerBuilder;
#[cfg(feature = "cold-buffer-std")]
pub use cold::ParallelCold;
#[cfg(feature = "cold-buffer")]
pub use cold::RingCold;
pub use cold::{ColdTier, DirectStorage, DirectStorageError, RecoverableColdTier};
pub use error::{BuilderError, ErasedPebbleManagerError, PebbleManagerError, Result};
pub use manifest::{Manifest, ManifestEntry};
pub use pebble_manager::PebbleManager;
pub use safety::{CapacityGuard, CheckpointRef};
#[cfg(feature = "bytecast")]
pub use serializers::BytecastSerializer;
pub use stats::{PebbleStats, TheoreticalValidation};
pub use traits::{CheckpointSerializer, Checkpointable};
pub use verify::VerificationResult;
pub use warm::{NoWarm, WarmCache, WarmTier};
