//! Native implementations for types not covered by zerocopy.
//!
//! Zerocopy cannot handle:
//! - `bool` / `char` - not all bit patterns are valid
//! - `usize` / `isize` - platform-dependent size
//! - `Option<T>` - discriminant + variable payload
//! - `Vec<T>` / `String` - variable length (alloc feature)

#[cfg(feature = "alloc")]
pub mod alloc;

#[cfg(feature = "std")]
pub mod std_impls;

pub mod tuple;
pub mod wrapper;

use crate::{BytesError, FromBytes, ToBytes};

// bool - needs validation (only 0 or 1 valid)
impl ToBytes for bool {
    const MAX_SIZE: Option<usize> = Some(1);

    #[inline]
    fn to_bytes(&self, buf: &mut [u8]) -> Result<usize, BytesError> {
        if buf.is_empty() {
            return Err(BytesError::BufferTooSmall {
                needed: 1,
                available: 0,
            });
        }
        buf[0] = *self as u8;
        Ok(1)
    }
}

impl FromBytes for bool {
    #[inline]
    fn from_bytes(buf: &[u8]) -> Result<(Self, usize), BytesError> {
        if buf.is_empty() {
            return Err(BytesError::UnexpectedEof {
                needed: 1,
                available: 0,
            });
        }
        match buf[0] {
            0 => Ok((false, 1)),
            1 => Ok((true, 1)),
            _ => Err(BytesError::InvalidData {
                message: "bool must be 0 or 1",
            }),
        }
    }
}

// char - needs validation (not all u32 values are valid)
impl ToBytes for char {
    const MAX_SIZE: Option<usize> = Some(4);

    #[inline]
    fn to_bytes(&self, buf: &mut [u8]) -> Result<usize, BytesError> {
        (*self as u32).to_bytes(buf)
    }
}

impl FromBytes for char {
    #[inline]
    fn from_bytes(buf: &[u8]) -> Result<(Self, usize), BytesError> {
        let (code, n) = u32::from_bytes(buf)?;
        let c = char::from_u32(code).ok_or(BytesError::InvalidData {
            message: "invalid char codepoint",
        })?;
        Ok((c, n))
    }
}

// usize - platform-dependent, serialize as u64 for portability
impl ToBytes for usize {
    const MAX_SIZE: Option<usize> = Some(8);

    #[inline]
    fn to_bytes(&self, buf: &mut [u8]) -> Result<usize, BytesError> {
        (*self as u64).to_bytes(buf)
    }
}

impl FromBytes for usize {
    #[inline]
    fn from_bytes(buf: &[u8]) -> Result<(Self, usize), BytesError> {
        let (v, n) = u64::from_bytes(buf)?;
        let val = usize::try_from(v).map_err(|_| BytesError::InvalidData {
            message: "u64 value exceeds usize on this platform",
        })?;
        Ok((val, n))
    }
}

// isize - platform-dependent, serialize as i64 for portability
impl ToBytes for isize {
    const MAX_SIZE: Option<usize> = Some(8);

    #[inline]
    fn to_bytes(&self, buf: &mut [u8]) -> Result<usize, BytesError> {
        (*self as i64).to_bytes(buf)
    }
}

impl FromBytes for isize {
    #[inline]
    fn from_bytes(buf: &[u8]) -> Result<(Self, usize), BytesError> {
        let (v, n) = i64::from_bytes(buf)?;
        let val = isize::try_from(v).map_err(|_| BytesError::InvalidData {
            message: "i64 value exceeds isize on this platform",
        })?;
        Ok((val, n))
    }
}

// Option<T> - zerocopy cannot handle this (discriminant + variable payload)
impl<T: ToBytes> ToBytes for Option<T> {
    const MAX_SIZE: Option<usize> = match T::MAX_SIZE {
        Some(s) => Some(1 + s),
        None => None,
    };

    fn to_bytes(&self, buf: &mut [u8]) -> Result<usize, BytesError> {
        match self {
            None => {
                if buf.is_empty() {
                    return Err(BytesError::BufferTooSmall {
                        needed: 1,
                        available: 0,
                    });
                }
                buf[0] = 0;
                Ok(1)
            }
            Some(v) => {
                if buf.is_empty() {
                    return Err(BytesError::BufferTooSmall {
                        needed: 1,
                        available: 0,
                    });
                }
                buf[0] = 1;
                let n = v.to_bytes(&mut buf[1..])?;
                Ok(1 + n)
            }
        }
    }
}

impl<T: FromBytes> FromBytes for Option<T> {
    fn from_bytes(buf: &[u8]) -> Result<(Self, usize), BytesError> {
        if buf.is_empty() {
            return Err(BytesError::UnexpectedEof {
                needed: 1,
                available: 0,
            });
        }
        match buf[0] {
            0 => Ok((None, 1)),
            1 => {
                let (v, n) = T::from_bytes(&buf[1..])?;
                Ok((Some(v), 1 + n))
            }
            _ => Err(BytesError::InvalidData {
                message: "Option discriminant must be 0 or 1",
            }),
        }
    }
}

// Result<T, E> - discriminant + variable payload (same pattern as Option)
const fn max_of(a: Option<usize>, b: Option<usize>) -> Option<usize> {
    match (a, b) {
        (Some(a), Some(b)) => {
            if a > b {
                Some(a)
            } else {
                Some(b)
            }
        }
        _ => None,
    }
}

impl<T: ToBytes, E: ToBytes> ToBytes for Result<T, E> {
    const MAX_SIZE: Option<usize> = match max_of(T::MAX_SIZE, E::MAX_SIZE) {
        Some(s) => Some(1 + s),
        None => None,
    };

    fn to_bytes(&self, buf: &mut [u8]) -> Result<usize, BytesError> {
        if buf.is_empty() {
            return Err(BytesError::BufferTooSmall {
                needed: 1,
                available: 0,
            });
        }
        match self {
            Ok(v) => {
                buf[0] = 0;
                let n = v.to_bytes(&mut buf[1..])?;
                Ok(1 + n)
            }
            Err(e) => {
                buf[0] = 1;
                let n = e.to_bytes(&mut buf[1..])?;
                Ok(1 + n)
            }
        }
    }

    fn byte_len(&self) -> Option<usize> {
        let inner = match self {
            Ok(v) => v.byte_len()?,
            Err(e) => e.byte_len()?,
        };
        Some(1 + inner)
    }
}

impl<T: FromBytes, E: FromBytes> FromBytes for Result<T, E> {
    fn from_bytes(buf: &[u8]) -> core::result::Result<(Self, usize), BytesError> {
        if buf.is_empty() {
            return Err(BytesError::UnexpectedEof {
                needed: 1,
                available: 0,
            });
        }
        match buf[0] {
            0 => {
                let (v, n) = T::from_bytes(&buf[1..])?;
                Ok((Ok(v), 1 + n))
            }
            1 => {
                let (e, n) = E::from_bytes(&buf[1..])?;
                Ok((Err(e), 1 + n))
            }
            _ => Err(BytesError::InvalidData {
                message: "Result discriminant must be 0 or 1",
            }),
        }
    }
}

// Range<T> - serialized as start then end
impl<T: ToBytes> ToBytes for core::ops::Range<T> {
    const MAX_SIZE: Option<usize> = match T::MAX_SIZE {
        Some(s) => Some(s * 2),
        None => None,
    };

    fn to_bytes(&self, buf: &mut [u8]) -> Result<usize, BytesError> {
        let n1 = self.start.to_bytes(buf)?;
        let n2 = self.end.to_bytes(&mut buf[n1..])?;
        Ok(n1 + n2)
    }

    fn byte_len(&self) -> Option<usize> {
        Some(self.start.byte_len()? + self.end.byte_len()?)
    }
}

impl<T: FromBytes> FromBytes for core::ops::Range<T> {
    fn from_bytes(buf: &[u8]) -> Result<(Self, usize), BytesError> {
        let (start, n1) = T::from_bytes(buf)?;
        let (end, n2) = T::from_bytes(&buf[n1..])?;
        Ok((start..end, n1 + n2))
    }
}

// RangeInclusive<T> - serialized as start then end
impl<T: ToBytes> ToBytes for core::ops::RangeInclusive<T> {
    const MAX_SIZE: Option<usize> = match T::MAX_SIZE {
        Some(s) => Some(s * 2),
        None => None,
    };

    fn to_bytes(&self, buf: &mut [u8]) -> Result<usize, BytesError> {
        let n1 = self.start().to_bytes(buf)?;
        let n2 = self.end().to_bytes(&mut buf[n1..])?;
        Ok(n1 + n2)
    }

    fn byte_len(&self) -> Option<usize> {
        Some(self.start().byte_len()? + self.end().byte_len()?)
    }
}

impl<T: FromBytes> FromBytes for core::ops::RangeInclusive<T> {
    fn from_bytes(buf: &[u8]) -> Result<(Self, usize), BytesError> {
        let (start, n1) = T::from_bytes(buf)?;
        let (end, n2) = T::from_bytes(&buf[n1..])?;
        Ok((start..=end, n1 + n2))
    }
}

// NonZero types - same wire format as inner, validated on decode
macro_rules! impl_nonzero {
    ($nz:ty, $inner:ty) => {
        impl ToBytes for $nz {
            const MAX_SIZE: Option<usize> = <$inner>::MAX_SIZE;

            #[inline]
            fn to_bytes(&self, buf: &mut [u8]) -> Result<usize, BytesError> {
                self.get().to_bytes(buf)
            }
        }

        impl FromBytes for $nz {
            #[inline]
            fn from_bytes(buf: &[u8]) -> Result<(Self, usize), BytesError> {
                let (v, n) = <$inner>::from_bytes(buf)?;
                let nz = <$nz>::new(v).ok_or(BytesError::InvalidData {
                    message: concat!(stringify!($nz), " must not be zero"),
                })?;
                Ok((nz, n))
            }
        }
    };
}

use core::num::{
    NonZeroI8, NonZeroI16, NonZeroI32, NonZeroI64, NonZeroI128, NonZeroU8, NonZeroU16, NonZeroU32,
    NonZeroU64, NonZeroU128,
};

impl_nonzero!(NonZeroU8, u8);
impl_nonzero!(NonZeroU16, u16);
impl_nonzero!(NonZeroU32, u32);
impl_nonzero!(NonZeroU64, u64);
impl_nonzero!(NonZeroU128, u128);
impl_nonzero!(NonZeroI8, i8);
impl_nonzero!(NonZeroI16, i16);
impl_nonzero!(NonZeroI32, i32);
impl_nonzero!(NonZeroI64, i64);
impl_nonzero!(NonZeroI128, i128);
