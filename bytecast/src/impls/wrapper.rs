//! Little-endian implementations for primitive types, unit, and arrays.
//!
//! All multi-byte primitives serialize as **little-endian** regardless of
//! platform, ensuring a deterministic and portable wire format.

use crate::{BytesError, FromBytes, ToBytes};

// Macro-generated LE impls for all numeric primitives.
macro_rules! impl_le_primitive {
    ($($ty:ty),+) => { $(
        impl ToBytes for $ty {
            const MAX_SIZE: Option<usize> = Some(core::mem::size_of::<$ty>());

            #[inline]
            fn to_bytes(&self, buf: &mut [u8]) -> Result<usize, BytesError> {
                const N: usize = core::mem::size_of::<$ty>();
                if buf.len() < N {
                    return Err(BytesError::BufferTooSmall { needed: N, available: buf.len() });
                }
                buf[..N].copy_from_slice(&self.to_le_bytes());
                Ok(N)
            }
        }

        impl FromBytes for $ty {
            #[inline]
            fn from_bytes(buf: &[u8]) -> Result<(Self, usize), BytesError> {
                const N: usize = core::mem::size_of::<$ty>();
                if buf.len() < N {
                    return Err(BytesError::UnexpectedEof { needed: N, available: buf.len() });
                }
                let arr: [u8; N] = buf[..N].try_into().unwrap();
                Ok((<$ty>::from_le_bytes(arr), N))
            }
        }
    )+ };
}

impl_le_primitive!(u8, u16, u32, u64, u128, i8, i16, i32, i64, i128, f32, f64);

// Unit type — zero-size, no bytes written or read.
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

// Arrays — element-by-element serialization.
impl<T: ToBytes, const N: usize> ToBytes for [T; N] {
    const MAX_SIZE: Option<usize> = match T::MAX_SIZE {
        Some(s) => Some(s * N),
        None => None,
    };

    fn to_bytes(&self, buf: &mut [u8]) -> Result<usize, BytesError> {
        let mut offset = 0;
        for item in self {
            offset += item.to_bytes(&mut buf[offset..])?;
        }
        Ok(offset)
    }

    fn byte_len(&self) -> Option<usize> {
        let mut total = 0;
        for item in self {
            total += item.byte_len()?;
        }
        Some(total)
    }
}

impl<T: FromBytes, const N: usize> FromBytes for [T; N] {
    fn from_bytes(buf: &[u8]) -> Result<(Self, usize), BytesError> {
        use core::mem::MaybeUninit;

        struct DropGuard<T, const N: usize> {
            arr: [MaybeUninit<T>; N],
            init: usize,
        }

        impl<T, const N: usize> Drop for DropGuard<T, N> {
            fn drop(&mut self) {
                for slot in &mut self.arr[..self.init] {
                    // SAFETY: elements [0..init) were written by `slot.write()`.
                    unsafe { slot.assume_init_drop() };
                }
            }
        }

        let mut guard = DropGuard::<T, N> {
            // SAFETY: An array of MaybeUninit<T> requires no initialization.
            arr: unsafe { MaybeUninit::uninit().assume_init() },
            init: 0,
        };
        let mut offset = 0;
        for i in 0..N {
            let (item, n) = T::from_bytes(&buf[offset..])?;
            guard.arr[i].write(item);
            guard.init += 1;
            offset += n;
        }
        // All N elements initialized — take ownership, inhibit guard's Drop.
        let arr = core::mem::replace(
            &mut guard.arr,
            // SAFETY: Replacing with uninit is fine — guard.init stays at N
            // but we immediately forget the guard so Drop never runs.
            unsafe { MaybeUninit::uninit().assume_init() },
        );
        core::mem::forget(guard);
        // SAFETY: All N elements were initialized in the loop above.
        let arr = arr.map(|slot| unsafe { slot.assume_init() });
        Ok((arr, offset))
    }
}
