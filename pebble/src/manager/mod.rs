//! Checkpoint management using Williams' pebble game algorithm.
//!
//! This module provides `PebbleManager`, a high-level interface for managing
//! checkpoints with O(√T) space complexity and near-optimal I/O operations.

mod branch;
mod branching;
mod builder;
pub mod cold;
mod error;
mod eviction;
mod manifest;
mod pebble_manager;
mod rebuild;
mod recovery;
mod safety;
mod stats;
mod traits;
mod verify;
pub mod warm;

pub use branch::{BranchError, BranchId, BranchInfo, HEAD};
pub use builder::{Missing, PebbleBuilder};
#[cfg(debug_assertions)]
pub use cold::DebugCold;
#[cfg(feature = "std")]
pub use cold::ParallelCold;
pub use cold::RingCold;
pub use cold::{ColdTier, DirectStorage, DirectStorageError, RecoverableColdTier};
pub use error::{ErasedPebbleManagerError, PebbleManagerError, Result};
pub use manifest::{Manifest, ManifestEntry};
pub use pebble_manager::PebbleManager;
pub use safety::{CapacityGuard, CheckpointRef};
pub use stats::{PebbleStats, TheoreticalValidation};
pub use traits::Checkpointable;
pub use verify::VerificationResult;
pub use warm::{NoWarm, WarmCache, WarmTier};
