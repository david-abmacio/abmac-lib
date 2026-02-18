use alloc::{
    borrow::{Cow, ToOwned},
    collections::VecDeque,
    string::String,
    vec::Vec,
};

use crate::{BytesError, FromBytes, ToBytes};

mod var_int {
    //! Variable-length integer encoding (LEB128-style).
    //!
    //! Encodes integers using 1-5 bytes depending on magnitude:
    //! - 0-127:         1 byte
    //! - 128-16383:     2 bytes
    //! - 16384-2097151: 3 bytes
    //! - etc.

    use crate::BytesError;

    /// Maximum bytes needed to encode a u32 as var_int.
    pub const MAX_VAR_INT_LEN: usize = 5;

    /// Encode a u32 as a var_int into the buffer. Returns bytes written.
    #[inline]
    pub fn encode(mut value: u32, buf: &mut [u8]) -> Result<usize, BytesError> {
        let mut i = 0;
        loop {
            if i >= buf.len() {
                return Err(BytesError::BufferTooSmall {
                    needed: i + 1,
                    available: buf.len(),
                });
            }
            if value < 0x80 {
                buf[i] = value as u8;
                return Ok(i + 1);
            }
            buf[i] = (value as u8 & 0x7F) | 0x80;
            value >>= 7;
            i += 1;
        }
    }

    /// Decode a var_int from the buffer. Returns (value, bytes consumed).
    #[inline]
    pub fn decode(buf: &[u8]) -> Result<(u32, usize), BytesError> {
        let (result, consumed) = decode_raw(buf)?;
        if consumed != len(result) {
            return Err(BytesError::InvalidData {
                message: "non-canonical var_int encoding",
            });
        }
        Ok((result, consumed))
    }

    fn decode_raw(buf: &[u8]) -> Result<(u32, usize), BytesError> {
        let mut result: u32 = 0;
        let mut shift = 0;

        for (i, &byte) in buf.iter().enumerate() {
            if i >= MAX_VAR_INT_LEN {
                return Err(BytesError::InvalidData {
                    message: "var_int too long",
                });
            }

            result |= ((byte & 0x7F) as u32) << shift;

            if byte < 0x80 {
                return Ok((result, i + 1));
            }

            shift += 7;
        }

        Err(BytesError::UnexpectedEof {
            needed: 1,
            available: 0,
        })
    }

    /// Calculate the number of bytes needed to encode a value as var_int.
    #[inline]
    pub const fn len(value: u32) -> usize {
        if value < (1 << 7) {
            1
        } else if value < (1 << 14) {
            2
        } else if value < (1 << 21) {
            3
        } else if value < (1 << 28) {
            4
        } else {
            5
        }
    }
}

/// Checked cast from usize to u32 for length prefixes.
#[inline]
fn checked_len(len: usize) -> Result<u32, BytesError> {
    u32::try_from(len).map_err(|_| BytesError::Custom {
        message: "collection exceeds u32::MAX length",
    })
}

impl<T: ToBytes> ToBytes for Vec<T> {
    const MAX_SIZE: Option<usize> = None; // Variable length

    fn to_bytes(&self, buf: &mut [u8]) -> Result<usize, BytesError> {
        // Length prefix as var_int (1-5 bytes depending on size)
        let len = checked_len(self.len())?;
        let mut offset = var_int::encode(len, buf)?;
        for item in self {
            offset += item.to_bytes(&mut buf[offset..])?;
        }
        Ok(offset)
    }

    fn byte_len(&self) -> Option<usize> {
        let mut total = var_int::len(checked_len(self.len()).ok()?);
        for item in self {
            total += item.byte_len()?;
        }
        Some(total)
    }
}

impl<T: FromBytes> FromBytes for Vec<T> {
    fn from_bytes(buf: &[u8]) -> Result<(Self, usize), BytesError> {
        let (len, mut offset) = var_int::decode(buf)?;
        let len = len as usize;
        let remaining = buf.len().saturating_sub(offset);
        if len > remaining {
            return Err(BytesError::UnexpectedEof {
                needed: offset + len,
                available: buf.len(),
            });
        }
        let mut vec = Vec::with_capacity(len);
        for _ in 0..len {
            let (item, n) = T::from_bytes(&buf[offset..])?;
            vec.push(item);
            offset += n;
        }
        Ok((vec, offset))
    }
}

// VecDeque<T> - same wire format as Vec<T>
impl<T: ToBytes> ToBytes for VecDeque<T> {
    const MAX_SIZE: Option<usize> = None;

    fn to_bytes(&self, buf: &mut [u8]) -> Result<usize, BytesError> {
        let len = checked_len(self.len())?;
        let mut offset = var_int::encode(len, buf)?;
        for item in self {
            offset += item.to_bytes(&mut buf[offset..])?;
        }
        Ok(offset)
    }

    fn byte_len(&self) -> Option<usize> {
        let mut total = var_int::len(checked_len(self.len()).ok()?);
        for item in self {
            total += item.byte_len()?;
        }
        Some(total)
    }
}

impl<T: FromBytes> FromBytes for VecDeque<T> {
    fn from_bytes(buf: &[u8]) -> Result<(Self, usize), BytesError> {
        let (len, mut offset) = var_int::decode(buf)?;
        let len = len as usize;
        let remaining = buf.len().saturating_sub(offset);
        if len > remaining {
            return Err(BytesError::UnexpectedEof {
                needed: offset + len,
                available: buf.len(),
            });
        }
        let mut deque = VecDeque::with_capacity(len);
        for _ in 0..len {
            let (item, n) = T::from_bytes(&buf[offset..])?;
            deque.push_back(item);
            offset += n;
        }
        Ok((deque, offset))
    }
}

impl ToBytes for String {
    const MAX_SIZE: Option<usize> = None;

    fn to_bytes(&self, buf: &mut [u8]) -> Result<usize, BytesError> {
        let bytes = self.as_bytes();
        // Length prefix as var_int (1-5 bytes depending on size)
        let len = checked_len(bytes.len())?;
        let offset = var_int::encode(len, buf)?;

        if buf.len().saturating_sub(offset) < bytes.len() {
            return Err(BytesError::BufferTooSmall {
                needed: offset + bytes.len(),
                available: buf.len(),
            });
        }
        buf[offset..offset + bytes.len()].copy_from_slice(bytes);
        Ok(offset + bytes.len())
    }

    fn byte_len(&self) -> Option<usize> {
        Some(var_int::len(checked_len(self.len()).ok()?) + self.len())
    }
}

impl FromBytes for String {
    fn from_bytes(buf: &[u8]) -> Result<(Self, usize), BytesError> {
        let (len, mut offset) = var_int::decode(buf)?;
        let len = len as usize;

        if buf.len().saturating_sub(offset) < len {
            return Err(BytesError::UnexpectedEof {
                needed: offset + len,
                available: buf.len(),
            });
        }

        let s = core::str::from_utf8(&buf[offset..offset + len])
            .map_err(|_| BytesError::InvalidData {
                message: "invalid UTF-8",
            })?
            .into();
        offset += len;
        Ok((s, offset))
    }
}

// Cow<'_, str> - same wire format as String
impl ToBytes for Cow<'_, str> {
    const MAX_SIZE: Option<usize> = None;

    #[inline]
    fn to_bytes(&self, buf: &mut [u8]) -> Result<usize, BytesError> {
        let bytes = self.as_bytes();
        let len = checked_len(bytes.len())?;
        let offset = var_int::encode(len, buf)?;

        if buf.len().saturating_sub(offset) < bytes.len() {
            return Err(BytesError::BufferTooSmall {
                needed: offset + bytes.len(),
                available: buf.len(),
            });
        }
        buf[offset..offset + bytes.len()].copy_from_slice(bytes);
        Ok(offset + bytes.len())
    }

    #[inline]
    fn byte_len(&self) -> Option<usize> {
        Some(var_int::len(checked_len(self.len()).ok()?) + self.len())
    }
}

impl FromBytes for Cow<'_, str> {
    #[inline]
    fn from_bytes(buf: &[u8]) -> Result<(Self, usize), BytesError> {
        let (s, n) = String::from_bytes(buf)?;
        Ok((Cow::Owned(s), n))
    }
}

// Cow<'_, [T]> - same wire format as Vec<T>
impl<T: ToBytes> ToBytes for Cow<'_, [T]>
where
    [T]: ToOwned,
{
    const MAX_SIZE: Option<usize> = None;

    #[inline]
    fn to_bytes(&self, buf: &mut [u8]) -> Result<usize, BytesError> {
        let slice: &[T] = self.as_ref();
        let len = checked_len(slice.len())?;
        let mut offset = var_int::encode(len, buf)?;
        for item in slice {
            offset += item.to_bytes(&mut buf[offset..])?;
        }
        Ok(offset)
    }

    #[inline]
    fn byte_len(&self) -> Option<usize> {
        let slice: &[T] = self.as_ref();
        let mut total = var_int::len(checked_len(slice.len()).ok()?);
        for item in slice {
            total += item.byte_len()?;
        }
        Some(total)
    }
}

impl<T: FromBytes + Clone> FromBytes for Cow<'_, [T]> {
    #[inline]
    fn from_bytes(buf: &[u8]) -> Result<(Self, usize), BytesError> {
        let (vec, n) = Vec::<T>::from_bytes(buf)?;
        Ok((Cow::Owned(vec), n))
    }
}
