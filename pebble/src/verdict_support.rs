//! Verdict integration: `Actionable` impls for all pebble error types.

use verdict::{Actionable, ErrorStatusValue};

use crate::dag::DAGError;
use crate::game::PebbleError;
use crate::manager::{BranchError, DirectStorageError, PebbleManagerError};
use crate::storage::StorageError;

impl Actionable for StorageError {
    fn status_value(&self) -> ErrorStatusValue {
        match self {
            StorageError::Io => ErrorStatusValue::Temporary,
            StorageError::Backend { .. } => ErrorStatusValue::Temporary,
            _ => ErrorStatusValue::Permanent,
        }
    }
}

impl Actionable for DAGError {
    fn status_value(&self) -> ErrorStatusValue {
        ErrorStatusValue::Permanent
    }
}

impl Actionable for PebbleError {
    fn status_value(&self) -> ErrorStatusValue {
        match self {
            PebbleError::FastMemoryExhausted { .. } => ErrorStatusValue::Temporary,
            _ => ErrorStatusValue::Permanent,
        }
    }
}

impl Actionable for DirectStorageError {
    fn status_value(&self) -> ErrorStatusValue {
        match self {
            DirectStorageError::Serializer { .. } => ErrorStatusValue::Permanent,
            DirectStorageError::Storage { source } => source.status_value(),
        }
    }
}

impl Actionable for BranchError {
    fn status_value(&self) -> ErrorStatusValue {
        ErrorStatusValue::Permanent
    }
}

impl<Id, E> Actionable for PebbleManagerError<Id, E>
where
    Id: core::fmt::Debug,
    E: core::fmt::Debug + core::fmt::Display + Actionable,
{
    fn status_value(&self) -> ErrorStatusValue {
        match self {
            PebbleManagerError::Storage { source } => source.status_value(),
            PebbleManagerError::DAG { source } => source.status_value(),
            PebbleManagerError::NeverAdded { .. } => ErrorStatusValue::Permanent,
            PebbleManagerError::StorageLoadFailed { .. } => ErrorStatusValue::Temporary,
            PebbleManagerError::DependencyMissing { .. } => ErrorStatusValue::Permanent,
            PebbleManagerError::Serialization { source, .. } => source.status_value(),
            PebbleManagerError::Deserialization { source, .. } => source.status_value(),
            PebbleManagerError::FlushFailed { source } => source.status_value(),
            PebbleManagerError::RebuildFailed { .. } => ErrorStatusValue::Permanent,
            PebbleManagerError::DependencyWidthExceeded { .. } => ErrorStatusValue::Permanent,
            PebbleManagerError::CheckpointTooLarge { .. } => ErrorStatusValue::Permanent,
            PebbleManagerError::InternalInconsistency { .. } => ErrorStatusValue::Permanent,
        }
    }
}
