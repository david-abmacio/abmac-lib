//! Error types for PebbleManager operations.

use alloc::string::String;

use crate::dag::DAGError;
use crate::storage::StorageError;

/// Error returned by [`PebbleManagerBuilder::build`](super::builder::PebbleManagerBuilder).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuilderError {
    /// `hot_capacity` was set to 0.
    ZeroHotCapacity,
}

impl core::fmt::Display for BuilderError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ZeroHotCapacity => write!(f, "hot_capacity must be at least 1"),
        }
    }
}

impl core::error::Error for BuilderError {}

/// Error type for PebbleManager operations.
///
/// # Type Parameters
/// - `Id` -- checkpoint identifier type, preserving the typed value from
///   [`Checkpointable::Id`](super::traits::Checkpointable).
/// - `E` -- serializer error type, preserving the original error from
///   [`CheckpointSerializer`](super::traits::CheckpointSerializer) for
///   programmatic inspection.
///
/// Use [`ErasedPebbleManagerError`] (alias for `PebbleManagerError<String, String>`)
/// when neither the typed ID nor the serializer error is needed.
#[must_use]
pub enum PebbleManagerError<Id, E> {
    Storage {
        source: StorageError,
    },
    DAG {
        source: DAGError,
    },
    NeverAdded {
        state_id: Id,
    },
    StorageLoadFailed {
        state_id: Id,
        reason: String,
    },
    DependencyMissing {
        dep_id: Id,
        for_id: Id,
    },
    Serialization {
        state_id: Id,
        source: E,
    },
    Deserialization {
        state_id: Id,
        source: E,
    },
    FlushFailed {
        source: E,
    },
    RebuildFailed {
        state_id: Id,
        reason: String,
    },
    DependencyWidthExceeded {
        state_id: Id,
        width: usize,
        limit: usize,
    },
    CheckpointTooLarge {
        size: usize,
        max: usize,
    },
    InternalInconsistency {
        detail: String,
    },
}

/// Type-erased PebbleManagerError using String for both IDs and serializer errors.
pub type ErasedPebbleManagerError = PebbleManagerError<String, String>;

/// Result type for PebbleManager operations.
pub type Result<T, Id, E> = core::result::Result<T, PebbleManagerError<Id, E>>;

// Debug

impl<Id: core::fmt::Debug, E: core::fmt::Debug> core::fmt::Debug for PebbleManagerError<Id, E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Storage { source } => f.debug_struct("Storage").field("source", source).finish(),
            Self::DAG { source } => f.debug_struct("DAG").field("source", source).finish(),
            Self::NeverAdded { state_id } => f
                .debug_struct("NeverAdded")
                .field("state_id", state_id)
                .finish(),
            Self::StorageLoadFailed { state_id, reason } => f
                .debug_struct("StorageLoadFailed")
                .field("state_id", state_id)
                .field("reason", reason)
                .finish(),
            Self::DependencyMissing { dep_id, for_id } => f
                .debug_struct("DependencyMissing")
                .field("dep_id", dep_id)
                .field("for_id", for_id)
                .finish(),
            Self::Serialization { state_id, source } => f
                .debug_struct("Serialization")
                .field("state_id", state_id)
                .field("source", source)
                .finish(),
            Self::Deserialization { state_id, source } => f
                .debug_struct("Deserialization")
                .field("state_id", state_id)
                .field("source", source)
                .finish(),
            Self::FlushFailed { source } => f
                .debug_struct("FlushFailed")
                .field("source", source)
                .finish(),
            Self::RebuildFailed { state_id, reason } => f
                .debug_struct("RebuildFailed")
                .field("state_id", state_id)
                .field("reason", reason)
                .finish(),
            Self::DependencyWidthExceeded {
                state_id,
                width,
                limit,
            } => f
                .debug_struct("DependencyWidthExceeded")
                .field("state_id", state_id)
                .field("width", width)
                .field("limit", limit)
                .finish(),
            Self::CheckpointTooLarge { size, max } => f
                .debug_struct("CheckpointTooLarge")
                .field("size", size)
                .field("max", max)
                .finish(),
            Self::InternalInconsistency { detail } => f
                .debug_struct("InternalInconsistency")
                .field("detail", detail)
                .finish(),
        }
    }
}

// Display

impl<Id: core::fmt::Debug, E: core::fmt::Display> core::fmt::Display for PebbleManagerError<Id, E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Storage { source } => write!(f, "storage error: {source}"),
            Self::DAG { source } => write!(f, "DAG error: {source}"),
            Self::NeverAdded { state_id } => {
                write!(f, "checkpoint never added: {state_id:?}")
            }
            Self::StorageLoadFailed { state_id, reason } => {
                write!(
                    f,
                    "storage load failed for checkpoint {state_id:?}: {reason}"
                )
            }
            Self::DependencyMissing { dep_id, for_id } => {
                write!(
                    f,
                    "dependency {dep_id:?} missing during rebuild of {for_id:?}"
                )
            }
            Self::Serialization { state_id, source } => {
                write!(
                    f,
                    "serialization failed for checkpoint {state_id:?}: {source}"
                )
            }
            Self::Deserialization { state_id, source } => {
                write!(
                    f,
                    "deserialization failed for checkpoint {state_id:?}: {source}"
                )
            }
            Self::FlushFailed { source } => {
                write!(f, "flush failed: {source}")
            }
            Self::RebuildFailed { state_id, reason } => {
                write!(f, "rebuild failed for state {state_id:?}: {reason}")
            }
            Self::DependencyWidthExceeded {
                state_id,
                width,
                limit,
            } => {
                write!(
                    f,
                    "cannot rebuild checkpoint {state_id:?}: dependency width \
                     {width} exceeds hot_capacity {limit}. Increase hot_capacity \
                     or restructure checkpoint dependencies."
                )
            }
            Self::CheckpointTooLarge { size, max } => {
                write!(f, "checkpoint size {size} exceeds maximum {max}")
            }
            Self::InternalInconsistency { detail } => {
                write!(f, "internal inconsistency: {detail}")
            }
        }
    }
}

// Error

impl<Id: core::fmt::Debug, E: core::fmt::Debug + core::fmt::Display> core::error::Error
    for PebbleManagerError<Id, E>
{
}

// Clone

impl<Id: Copy, E: Clone> Clone for PebbleManagerError<Id, E> {
    fn clone(&self) -> Self {
        match self {
            Self::Storage { source } => Self::Storage {
                source: source.clone(),
            },
            Self::DAG { source } => Self::DAG {
                source: source.clone(),
            },
            Self::NeverAdded { state_id } => Self::NeverAdded {
                state_id: *state_id,
            },
            Self::StorageLoadFailed { state_id, reason } => Self::StorageLoadFailed {
                state_id: *state_id,
                reason: reason.clone(),
            },
            Self::DependencyMissing { dep_id, for_id } => Self::DependencyMissing {
                dep_id: *dep_id,
                for_id: *for_id,
            },
            Self::Serialization { state_id, source } => Self::Serialization {
                state_id: *state_id,
                source: source.clone(),
            },
            Self::Deserialization { state_id, source } => Self::Deserialization {
                state_id: *state_id,
                source: source.clone(),
            },
            Self::FlushFailed { source } => Self::FlushFailed {
                source: source.clone(),
            },
            Self::RebuildFailed { state_id, reason } => Self::RebuildFailed {
                state_id: *state_id,
                reason: reason.clone(),
            },
            Self::DependencyWidthExceeded {
                state_id,
                width,
                limit,
            } => Self::DependencyWidthExceeded {
                state_id: *state_id,
                width: *width,
                limit: *limit,
            },
            Self::CheckpointTooLarge { size, max } => Self::CheckpointTooLarge {
                size: *size,
                max: *max,
            },
            Self::InternalInconsistency { detail } => Self::InternalInconsistency {
                detail: detail.clone(),
            },
        }
    }
}

// PartialEq / Eq

impl<Id: PartialEq, E: PartialEq> PartialEq for PebbleManagerError<Id, E> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Storage { source: a }, Self::Storage { source: b }) => a == b,
            (Self::DAG { source: a }, Self::DAG { source: b }) => a == b,
            (Self::NeverAdded { state_id: a }, Self::NeverAdded { state_id: b }) => a == b,
            (
                Self::StorageLoadFailed {
                    state_id: a1,
                    reason: a2,
                },
                Self::StorageLoadFailed {
                    state_id: b1,
                    reason: b2,
                },
            ) => a1 == b1 && a2 == b2,
            (
                Self::DependencyMissing {
                    dep_id: a1,
                    for_id: a2,
                },
                Self::DependencyMissing {
                    dep_id: b1,
                    for_id: b2,
                },
            ) => a1 == b1 && a2 == b2,
            (
                Self::Serialization {
                    state_id: a1,
                    source: a2,
                },
                Self::Serialization {
                    state_id: b1,
                    source: b2,
                },
            ) => a1 == b1 && a2 == b2,
            (
                Self::Deserialization {
                    state_id: a1,
                    source: a2,
                },
                Self::Deserialization {
                    state_id: b1,
                    source: b2,
                },
            ) => a1 == b1 && a2 == b2,
            (Self::FlushFailed { source: a }, Self::FlushFailed { source: b }) => a == b,
            (
                Self::RebuildFailed {
                    state_id: a1,
                    reason: a2,
                },
                Self::RebuildFailed {
                    state_id: b1,
                    reason: b2,
                },
            ) => a1 == b1 && a2 == b2,
            (
                Self::DependencyWidthExceeded {
                    state_id: a1,
                    width: a2,
                    limit: a3,
                },
                Self::DependencyWidthExceeded {
                    state_id: b1,
                    width: b2,
                    limit: b3,
                },
            ) => a1 == b1 && a2 == b2 && a3 == b3,
            (
                Self::CheckpointTooLarge { size: a1, max: a2 },
                Self::CheckpointTooLarge { size: b1, max: b2 },
            ) => a1 == b1 && a2 == b2,
            (
                Self::InternalInconsistency { detail: a },
                Self::InternalInconsistency { detail: b },
            ) => a == b,
            _ => false,
        }
    }
}

impl<Id: Eq, E: Eq> Eq for PebbleManagerError<Id, E> {}

// From conversions

impl<Id, E> From<StorageError> for PebbleManagerError<Id, E> {
    fn from(source: StorageError) -> Self {
        PebbleManagerError::Storage { source }
    }
}

impl<Id, E> From<DAGError> for PebbleManagerError<Id, E> {
    fn from(source: DAGError) -> Self {
        PebbleManagerError::DAG { source }
    }
}

// Erasure

impl<Id: core::fmt::Debug, E: core::fmt::Display> PebbleManagerError<Id, E> {
    /// Convert to a type-erased error, formatting the checkpoint ID via Debug
    /// and the serializer error via Display.
    pub fn erase(self) -> ErasedPebbleManagerError {
        match self {
            Self::Serialization { state_id, source } => PebbleManagerError::Serialization {
                state_id: alloc::format!("{state_id:?}"),
                source: alloc::format!("{source}"),
            },
            Self::Deserialization { state_id, source } => PebbleManagerError::Deserialization {
                state_id: alloc::format!("{state_id:?}"),
                source: alloc::format!("{source}"),
            },
            Self::FlushFailed { source } => PebbleManagerError::FlushFailed {
                source: alloc::format!("{source}"),
            },
            Self::Storage { source } => PebbleManagerError::Storage { source },
            Self::DAG { source } => PebbleManagerError::DAG { source },
            Self::NeverAdded { state_id } => PebbleManagerError::NeverAdded {
                state_id: alloc::format!("{state_id:?}"),
            },
            Self::StorageLoadFailed { state_id, reason } => PebbleManagerError::StorageLoadFailed {
                state_id: alloc::format!("{state_id:?}"),
                reason,
            },
            Self::DependencyMissing { dep_id, for_id } => PebbleManagerError::DependencyMissing {
                dep_id: alloc::format!("{dep_id:?}"),
                for_id: alloc::format!("{for_id:?}"),
            },
            Self::RebuildFailed { state_id, reason } => PebbleManagerError::RebuildFailed {
                state_id: alloc::format!("{state_id:?}"),
                reason,
            },
            Self::DependencyWidthExceeded {
                state_id,
                width,
                limit,
            } => PebbleManagerError::DependencyWidthExceeded {
                state_id: alloc::format!("{state_id:?}"),
                width,
                limit,
            },
            Self::CheckpointTooLarge { size, max } => {
                PebbleManagerError::CheckpointTooLarge { size, max }
            }
            Self::InternalInconsistency { detail } => {
                PebbleManagerError::InternalInconsistency { detail }
            }
        }
    }
}
