//! Implementations of ToBytes/FromBytes for primitive types.

use crate::bytes::{BytesError, FromBytes, ToBytes};

// u8 implementation (special case - no endianness)
impl ToBytes for u8 {
    const MAX_SIZE: Option<usize> = Some(1);

    #[inline]
    fn to_bytes(&self, buf: &mut [u8]) -> Result<usize, BytesError> {
        if buf.is_empty() {
            return Err(BytesError::BufferTooSmall {
                needed: 1,
                available: 0,
            });
        }
        buf[0] = *self;
        Ok(1)
    }
}

impl FromBytes for u8 {
    #[inline]
    fn from_bytes(buf: &[u8]) -> Result<(Self, usize), BytesError> {
        if buf.is_empty() {
            return Err(BytesError::UnexpectedEof {
                needed: 1,
                available: 0,
            });
        }
        Ok((buf[0], 1))
    }
}

// i8 implementation (special case - no endianness)
impl ToBytes for i8 {
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

impl FromBytes for i8 {
    #[inline]
    fn from_bytes(buf: &[u8]) -> Result<(Self, usize), BytesError> {
        if buf.is_empty() {
            return Err(BytesError::UnexpectedEof {
                needed: 1,
                available: 0,
            });
        }
        Ok((buf[0] as i8, 1))
    }
}

// bool implementation
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

// Macro for multi-byte integer types
macro_rules! impl_bytes_for_int {
    ($($ty:ty),+) => {
        $(
            impl ToBytes for $ty {
                const MAX_SIZE: Option<usize> = Some(core::mem::size_of::<$ty>());

                #[inline]
                fn to_bytes(&self, buf: &mut [u8]) -> Result<usize, BytesError> {
                    const SIZE: usize = core::mem::size_of::<$ty>();
                    if buf.len() < SIZE {
                        return Err(BytesError::BufferTooSmall {
                            needed: SIZE,
                            available: buf.len(),
                        });
                    }
                    buf[..SIZE].copy_from_slice(&self.to_le_bytes());
                    Ok(SIZE)
                }
            }

            impl FromBytes for $ty {
                #[inline]
                fn from_bytes(buf: &[u8]) -> Result<(Self, usize), BytesError> {
                    const SIZE: usize = core::mem::size_of::<$ty>();
                    if buf.len() < SIZE {
                        return Err(BytesError::UnexpectedEof {
                            needed: SIZE,
                            available: buf.len(),
                        });
                    }
                    // SAFETY: We verified buf.len() >= SIZE above, so this slice
                    // is exactly SIZE bytes and try_into() cannot fail.
                    let Ok(bytes) = buf[..SIZE].try_into() else {
                        unreachable!()
                    };
                    Ok((<$ty>::from_le_bytes(bytes), SIZE))
                }
            }
        )+
    };
}

impl_bytes_for_int!(u16, u32, u64, u128, i16, i32, i64, i128);

// usize/isize - serialize as u64/i64 for portability
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
        Ok((v as usize, n))
    }
}

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
        Ok((v as isize, n))
    }
}

// Floating point types
impl ToBytes for f32 {
    const MAX_SIZE: Option<usize> = Some(4);

    #[inline]
    fn to_bytes(&self, buf: &mut [u8]) -> Result<usize, BytesError> {
        self.to_bits().to_bytes(buf)
    }
}

impl FromBytes for f32 {
    #[inline]
    fn from_bytes(buf: &[u8]) -> Result<(Self, usize), BytesError> {
        let (bits, n) = u32::from_bytes(buf)?;
        Ok((f32::from_bits(bits), n))
    }
}

impl ToBytes for f64 {
    const MAX_SIZE: Option<usize> = Some(8);

    #[inline]
    fn to_bytes(&self, buf: &mut [u8]) -> Result<usize, BytesError> {
        self.to_bits().to_bytes(buf)
    }
}

impl FromBytes for f64 {
    #[inline]
    fn from_bytes(buf: &[u8]) -> Result<(Self, usize), BytesError> {
        let (bits, n) = u64::from_bytes(buf)?;
        Ok((f64::from_bits(bits), n))
    }
}

// Unit type
impl ToBytes for () {
    const MAX_SIZE: Option<usize> = Some(0);

    #[inline]
    fn to_bytes(&self, _buf: &mut [u8]) -> Result<usize, BytesError> {
        Ok(0)
    }
}

impl FromBytes for () {
    #[inline]
    fn from_bytes(_buf: &[u8]) -> Result<(Self, usize), BytesError> {
        Ok(((), 0))
    }
}
