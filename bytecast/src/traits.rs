//! Byte serialization traits.

use crate::BytesError;

/// Serialize a value to a caller-provided buffer.
pub trait ToBytes {
    /// Maximum serialized size, if known at compile time.
    const MAX_SIZE: Option<usize> = None;

    /// Serialize into the buffer. Returns bytes written.
    fn to_bytes(&self, buf: &mut [u8]) -> Result<usize, BytesError>;

    /// Runtime size hint for pre-allocation.
    #[inline]
    fn byte_len(&self) -> Option<usize> {
        Self::MAX_SIZE
    }
}

/// Deserialize from bytes to an owned value.
pub trait FromBytes: Sized {
    /// Deserialize from bytes. Returns value and bytes consumed.
    fn from_bytes(buf: &[u8]) -> Result<(Self, usize), BytesError>;
}

/// Zero-copy view into serialized bytes.
pub trait ViewBytes<'a>: Sized {
    /// Create a view into the bytes without copying.
    fn view(bytes: &'a [u8]) -> Result<Self, BytesError>;
}
