use crate::BytesError;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

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

/// Convenience methods for ToBytes types.
pub trait ToBytesExt: ToBytes {
    /// Serialize to a new Vec.
    #[cfg(feature = "alloc")]
    fn to_vec(&self) -> Result<Vec<u8>, BytesError> {
        let size = self.byte_len().or(Self::MAX_SIZE).unwrap_or(256);
        let mut buf = alloc::vec![0u8; size];
        let n = self.to_bytes(&mut buf)?;
        buf.truncate(n);
        Ok(buf)
    }

    /// Serialize to a fixed-size array.
    fn to_array<const N: usize>(&self) -> Result<[u8; N], BytesError> {
        let mut arr = [0u8; N];
        let n = self.to_bytes(&mut arr)?;
        if n != N {
            return Err(BytesError::Custom {
                message: "size mismatch",
            });
        }
        Ok(arr)
    }
}

impl<T: ToBytes> ToBytesExt for T {}

/// Convenience methods for FromBytes types.
pub trait FromBytesExt: FromBytes {
    /// Deserialize, ignoring trailing bytes.
    fn from_bytes_partial(buf: &[u8]) -> Result<Self, BytesError> {
        let (v, _) = Self::from_bytes(buf)?;
        Ok(v)
    }

    /// Deserialize, requiring exact buffer consumption.
    fn from_bytes_exact(buf: &[u8]) -> Result<Self, BytesError> {
        let (v, n) = Self::from_bytes(buf)?;
        if n != buf.len() {
            return Err(BytesError::Custom {
                message: "trailing bytes",
            });
        }
        Ok(v)
    }
}

impl<T: FromBytes> FromBytesExt for T {}
