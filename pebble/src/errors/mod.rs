//! Centralized error types for the pebble crate.

pub mod dag;
pub mod game;
pub mod manager;
pub mod storage;

pub use dag::DAGError;
pub use game::PebbleError;
pub use manager::{
    BranchError, BuilderError, DirectStorageError, ErasedPebbleManagerError, PebbleManagerError,
};
pub use storage::{IntegrityError, IntegrityErrorKind, StorageError};
