//! Generic serializer using ToBytes/FromBytes traits.

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use super::{BytesError, FromBytes, ToBytes};

/// A serializer that uses ToBytes/FromBytes traits.
///
/// This provides a simple way to serialize types that implement the byte traits
/// into owned Vec<u8> buffers.
#[derive(Debug, Clone, Copy, Default)]
pub struct ByteSerializer;

impl ByteSerializer {
    /// Create a new ByteSerializer.
    pub const fn new() -> Self {
        Self
    }

    /// Serialize a value to bytes.
    pub fn serialize<T: ToBytes>(self, value: &T) -> Result<Vec<u8>, BytesError> {
        let size = value.byte_len().or(T::MAX_SIZE).unwrap_or(256);
        let mut buf = vec![0u8; size];
        let written = value.to_bytes(&mut buf)?;
        buf.truncate(written);
        Ok(buf)
    }

    /// Deserialize a value from bytes.
    pub fn deserialize<T: FromBytes>(self, bytes: &[u8]) -> Result<T, BytesError> {
        let (value, _) = T::from_bytes(bytes)?;
        Ok(value)
    }
}
