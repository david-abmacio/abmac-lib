//! Error types for byte serialization.

use snafu::Snafu;

/// Error during byte serialization or deserialization.
#[derive(Debug, Clone, PartialEq, Eq, Snafu)]
#[snafu(visibility(pub))]
pub enum BytesError {
    /// Buffer too small for serialization.
    #[snafu(display("buffer too small: needed {needed} bytes, only {available} available"))]
    BufferTooSmall {
        /// Bytes needed.
        needed: usize,
        /// Bytes available.
        available: usize,
    },

    /// Invalid data during deserialization.
    #[snafu(display("invalid data: {message}"))]
    InvalidData {
        /// Error description.
        message: &'static str,
    },

    /// Unexpected end of input.
    #[snafu(display("unexpected end of input: needed {needed} bytes, only {available} available"))]
    UnexpectedEof {
        /// Bytes needed.
        needed: usize,
        /// Bytes available.
        available: usize,
    },

    /// Custom error for user implementations.
    #[snafu(display("{message}"))]
    Custom {
        /// Error description.
        message: &'static str,
    },
}

/// Result type for bytes operations.
pub type Result<T> = core::result::Result<T, BytesError>;
