#[cfg(feature = "alloc")]
mod alloc;

#[cfg(feature = "std")]
mod std_collections;

#[cfg(feature = "serde")]
mod serde;

#[cfg(feature = "facet")]
mod facet;

#[cfg(feature = "rkyv")]
mod rkyv;

use core::{f32, f64};

use super::*;

#[test]
fn test_u8() {
    let mut buf = [0u8; 1];
    assert_eq!(42u8.to_bytes(&mut buf).unwrap(), 1);
    assert_eq!(buf[0], 42);

    let (v, n) = u8::from_bytes(&buf).unwrap();
    assert_eq!(v, 42);
    assert_eq!(n, 1);
}

#[test]
fn test_i8() {
    let mut buf = [0u8; 1];
    assert_eq!((-42i8).to_bytes(&mut buf).unwrap(), 1);

    let (v, n) = i8::from_bytes(&buf).unwrap();
    assert_eq!(v, -42);
    assert_eq!(n, 1);
}

#[test]
fn test_bool() {
    let mut buf = [0u8; 1];

    true.to_bytes(&mut buf).unwrap();
    assert_eq!(buf[0], 1);
    let (v, _) = bool::from_bytes(&buf).unwrap();
    assert!(v);

    false.to_bytes(&mut buf).unwrap();
    assert_eq!(buf[0], 0);
    let (v, _) = bool::from_bytes(&buf).unwrap();
    assert!(!v);
}

#[test]
fn test_bool_invalid() {
    let buf = [2u8];
    assert!(matches!(
        bool::from_bytes(&buf),
        Err(BytesError::InvalidData { .. })
    ));
}

#[test]
fn test_u32() {
    let mut buf = [0u8; 4];
    let value = 0x12345678u32;

    assert_eq!(value.to_bytes(&mut buf).unwrap(), 4);
    // Little-endian
    assert_eq!(buf, [0x78, 0x56, 0x34, 0x12]);

    let (v, n) = u32::from_bytes(&buf).unwrap();
    assert_eq!(v, value);
    assert_eq!(n, 4);
}

#[test]
fn test_i32_negative() {
    let mut buf = [0u8; 4];
    let value = -12345i32;

    value.to_bytes(&mut buf).unwrap();
    let (v, _) = i32::from_bytes(&buf).unwrap();
    assert_eq!(v, value);
}

#[test]
fn test_u64() {
    let mut buf = [0u8; 8];
    let value = 0x123456789ABCDEF0u64;

    assert_eq!(value.to_bytes(&mut buf).unwrap(), 8);
    let (v, n) = u64::from_bytes(&buf).unwrap();
    assert_eq!(v, value);
    assert_eq!(n, 8);
}

#[test]
fn test_u128() {
    let mut buf = [0u8; 16];
    let value = 0x123456789ABCDEF0_FEDCBA9876543210u128;

    assert_eq!(value.to_bytes(&mut buf).unwrap(), 16);
    let (v, n) = u128::from_bytes(&buf).unwrap();
    assert_eq!(v, value);
    assert_eq!(n, 16);
}

#[test]
fn test_f32() {
    let mut buf = [0u8; 4];
    let value = f32::consts::PI;

    value.to_bytes(&mut buf).unwrap();
    let (v, _) = f32::from_bytes(&buf).unwrap();
    assert_eq!(v, value);
}

#[test]
fn test_f64() {
    let mut buf = [0u8; 8];
    let value = f64::consts::PI;

    value.to_bytes(&mut buf).unwrap();
    let (v, _) = f64::from_bytes(&buf).unwrap();
    assert_eq!(v, value);
}

#[test]
fn test_unit() {
    let mut buf = [0u8; 0];
    assert_eq!(().to_bytes(&mut buf).unwrap(), 0);
    let (v, n) = <()>::from_bytes(&buf).unwrap();
    assert_eq!(v, ());
    assert_eq!(n, 0);
}

#[test]
fn test_buffer_too_small() {
    let mut buf = [0u8; 2];
    let result = 42u32.to_bytes(&mut buf);
    assert!(matches!(
        result,
        Err(BytesError::BufferTooSmall {
            needed: 4,
            available: 2
        })
    ));
}

#[test]
fn test_unexpected_eof() {
    let buf = [0u8; 2];
    let result = u32::from_bytes(&buf);
    assert!(matches!(
        result,
        Err(BytesError::UnexpectedEof {
            needed: 4,
            available: 2
        })
    ));
}

#[test]
fn test_max_size() {
    assert_eq!(u8::MAX_SIZE, Some(1));
    assert_eq!(u16::MAX_SIZE, Some(2));
    assert_eq!(u32::MAX_SIZE, Some(4));
    assert_eq!(u64::MAX_SIZE, Some(8));
    assert_eq!(u128::MAX_SIZE, Some(16));
    assert_eq!(bool::MAX_SIZE, Some(1));
    assert_eq!(f32::MAX_SIZE, Some(4));
    assert_eq!(f64::MAX_SIZE, Some(8));
    assert_eq!(<()>::MAX_SIZE, Some(0));
}

#[test]
fn test_byte_len() {
    assert_eq!(42u32.byte_len(), Some(4));
    assert_eq!(true.byte_len(), Some(1));
}

#[test]
fn test_usize_portability() {
    // usize serializes as u64 for cross-platform compatibility
    let mut buf = [0u8; 8];
    let value: usize = 12345;

    assert_eq!(value.to_bytes(&mut buf).unwrap(), 8);
    let (v, n) = usize::from_bytes(&buf).unwrap();
    assert_eq!(v, value);
    assert_eq!(n, 8);
}

#[test]
fn test_isize_portability() {
    // isize serializes as i64 for cross-platform compatibility
    let mut buf = [0u8; 8];
    let value: isize = -12345;

    assert_eq!(value.to_bytes(&mut buf).unwrap(), 8);
    let (v, n) = isize::from_bytes(&buf).unwrap();
    assert_eq!(v, value);
    assert_eq!(n, 8);
}

// char tests
#[test]
fn test_char_ascii() {
    let mut buf = [0u8; 4];
    let value = 'A';

    let written = value.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 4);

    let (decoded, consumed) = char::from_bytes(&buf).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(consumed, 4);
}

#[test]
fn test_char_unicode() {
    let mut buf = [0u8; 4];
    let value = '🦀';

    value.to_bytes(&mut buf).unwrap();
    let (decoded, _) = char::from_bytes(&buf).unwrap();
    assert_eq!(decoded, value);
}

#[test]
fn test_char_invalid() {
    // 0xD800 is an invalid Unicode scalar value (surrogate)
    let buf = [0x00, 0xD8, 0x00, 0x00]; // 0xD800 in little-endian
    let result = char::from_bytes(&buf);
    assert!(matches!(result, Err(BytesError::InvalidData { .. })));
}

// Option tests
#[test]
fn test_option_some() {
    let mut buf = [0u8; 8];
    let value: Option<u32> = Some(0x12345678);

    let written = value.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 5); // 1 byte discriminant + 4 bytes u32

    let (decoded, consumed) = Option::<u32>::from_bytes(&buf).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(consumed, written);
}

#[test]
fn test_option_none() {
    let mut buf = [0u8; 8];
    let value: Option<u32> = None;

    let written = value.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 1); // just discriminant

    let (decoded, consumed) = Option::<u32>::from_bytes(&buf).unwrap();
    assert_eq!(decoded, None);
    assert_eq!(consumed, 1);
}

#[test]
fn test_option_invalid_discriminant() {
    let buf = [2u8, 0, 0, 0, 0]; // invalid discriminant
    let result = Option::<u32>::from_bytes(&buf);
    assert!(matches!(result, Err(BytesError::InvalidData { .. })));
}

#[test]
fn test_array_roundtrip() {
    let mut buf = [0u8; 16];
    let value: [u32; 4] = [1, 2, 3, 4];

    let written = value.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 16);

    let (decoded, consumed) = <[u32; 4]>::from_bytes(&buf).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(consumed, 16);
}

// Tuple tests
#[test]
fn test_tuple_fixed() {
    let mut buf = [0u8; 5];
    let value: (u32, u8) = (0x12345678, 42);

    let written = value.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 5);

    let (decoded, consumed) = <(u32, u8)>::from_bytes(&buf).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(consumed, 5);

    assert_eq!(<(u32, u8)>::MAX_SIZE, Some(5));
    assert_eq!(value.byte_len(), Some(5));
}

#[test]
fn test_tuple_single() {
    let mut buf = [0u8; 4];
    let value: (u32,) = (99,);

    let written = value.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 4);

    let (decoded, consumed) = <(u32,)>::from_bytes(&buf).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(consumed, 4);
}

#[test]
fn test_tuple_arity_12() {
    let mut buf = [0u8; 12];
    let value: (u8, u8, u8, u8, u8, u8, u8, u8, u8, u8, u8, u8) =
        (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12);

    let written = value.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 12);

    let (decoded, consumed) =
        <(u8, u8, u8, u8, u8, u8, u8, u8, u8, u8, u8, u8)>::from_bytes(&buf).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(consumed, 12);
}

// Result tests
#[test]
fn test_result_ok() {
    let mut buf = [0u8; 8];
    let value: core::result::Result<u32, u8> = Ok(0x12345678);

    let written = value.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 5); // 1 byte discriminant + 4 bytes u32

    let (decoded, consumed) = <core::result::Result<u32, u8>>::from_bytes(&buf).unwrap();
    assert_eq!(decoded, Ok(0x12345678));
    assert_eq!(consumed, 5);
}

#[test]
fn test_result_err() {
    let mut buf = [0u8; 8];
    let value: core::result::Result<u32, u8> = Err(42);

    let written = value.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 2); // 1 byte discriminant + 1 byte u8

    let (decoded, consumed) = <core::result::Result<u32, u8>>::from_bytes(&buf).unwrap();
    assert_eq!(decoded, Err(42));
    assert_eq!(consumed, 2);
}

#[test]
fn test_result_invalid_discriminant() {
    let buf = [2u8, 0, 0, 0, 0];
    let result = <core::result::Result<u32, u8>>::from_bytes(&buf);
    assert!(matches!(result, Err(BytesError::InvalidData { .. })));
}

#[test]
fn test_result_max_size() {
    // max(4, 1) + 1 = 5
    assert_eq!(<core::result::Result<u32, u8>>::MAX_SIZE, Some(5));
    // max(4, 4) + 1 = 5
    assert_eq!(<core::result::Result<u32, u32>>::MAX_SIZE, Some(5));
    // max(1, 8) + 1 = 9
    assert_eq!(<core::result::Result<u8, u64>>::MAX_SIZE, Some(9));
}

// Range tests
#[test]
fn test_range_roundtrip() {
    let mut buf = [0u8; 8];
    let value: core::ops::Range<u32> = 10..42;
    let written = value.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 8);
    let (decoded, consumed) = <core::ops::Range<u32>>::from_bytes(&buf).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(consumed, 8);
}

#[test]
fn test_range_max_size() {
    assert_eq!(<core::ops::Range<u32>>::MAX_SIZE, Some(8));
    assert_eq!(<core::ops::Range<u8>>::MAX_SIZE, Some(2));
}

#[test]
fn test_range_inclusive_roundtrip() {
    let mut buf = [0u8; 8];
    let value: core::ops::RangeInclusive<u32> = 10..=42;
    let written = value.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 8);
    let (decoded, consumed) = <core::ops::RangeInclusive<u32>>::from_bytes(&buf).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(consumed, 8);
}

#[test]
fn test_range_inclusive_max_size() {
    assert_eq!(<core::ops::RangeInclusive<u32>>::MAX_SIZE, Some(8));
}

// NonZero tests
#[test]
fn test_nonzero_u32_roundtrip() {
    use core::num::NonZeroU32;
    let mut buf = [0u8; 4];
    let value = NonZeroU32::new(42).unwrap();
    let written = value.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 4);
    let (decoded, consumed) = NonZeroU32::from_bytes(&buf).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(consumed, 4);
}

#[test]
fn test_nonzero_u32_rejects_zero() {
    use core::num::NonZeroU32;
    let buf = [0u8; 4];
    let result = NonZeroU32::from_bytes(&buf);
    assert!(matches!(result, Err(BytesError::InvalidData { .. })));
}

#[test]
fn test_nonzero_i64_roundtrip() {
    use core::num::NonZeroI64;
    let mut buf = [0u8; 8];
    let value = NonZeroI64::new(-99).unwrap();
    let written = value.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 8);
    let (decoded, consumed) = NonZeroI64::from_bytes(&buf).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(consumed, 8);
}

#[test]
fn test_nonzero_u8_max_size() {
    use core::num::NonZeroU8;
    assert_eq!(NonZeroU8::MAX_SIZE, Some(1));
}

#[test]
fn test_nonzero_u128_roundtrip() {
    use core::num::NonZeroU128;
    let mut buf = [0u8; 16];
    let value = NonZeroU128::new(1).unwrap();
    let written = value.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 16);
    let (decoded, consumed) = NonZeroU128::from_bytes(&buf).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(consumed, 16);
}
