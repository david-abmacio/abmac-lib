//! Cold-storage error types.

use crate::errors::storage::StorageError;

verdict::display_error! {
    /// Error type for [`DirectStorage`](crate::manager::cold::DirectStorage) operations.
    ///
    /// Wraps either a serialization/deserialization error or a storage error.
    pub enum DirectStorageError {
        #[display("serializer error: {source}")]
        Serializer { source: bytecast::BytesError },

        #[display("storage error: {source}")]
        Storage { source: StorageError },
    }
}

impl From<StorageError> for DirectStorageError {
    fn from(e: StorageError) -> Self {
        Self::Storage { source: e }
    }
}
