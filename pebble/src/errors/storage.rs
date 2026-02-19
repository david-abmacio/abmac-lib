//! Storage-related error types.

verdict::display_error! {
    #[derive(Clone, PartialEq, Eq)]
    pub enum StorageError {
        #[display("checkpoint not found")]
        NotFound,

        #[display("checksum mismatch: expected {expected:#x}, got {actual:#x}")]
        ChecksumMismatch { expected: u32, actual: u32 },

        #[display("buffer too small: need {required} bytes, got {provided}")]
        BufferTooSmall { required: usize, provided: usize },

        #[display("I/O error")]
        Io,

        #[display("backend error: {message}")]
        Backend { message: &'static str },

        #[display("too many dependencies: maximum {max}, got {count}")]
        TooManyDependencies { max: usize, count: usize },
    }
}

/// Checkpoint validation error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrityError {
    pub state_id: alloc::string::String,
    pub kind: IntegrityErrorKind,
}

/// Types of integrity errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegrityErrorKind {
    ChecksumMismatch { expected: u32, actual: u32 },
    MissingDependency { dep_id: alloc::string::String },
    DeserializationFailed,
}
