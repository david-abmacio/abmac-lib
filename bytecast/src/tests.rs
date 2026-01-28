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

// ByteSerializer tests

use super::ByteSerializer;

#[test]
fn test_byte_serializer_new() {
    let _serializer = ByteSerializer::new();
    let _default = ByteSerializer::default();
}

#[test]
fn test_byte_serializer_roundtrip_u32() {
    let serializer = ByteSerializer::new();
    let value = 0x12345678u32;

    let bytes = serializer.serialize(&value).unwrap();
    assert_eq!(bytes.len(), 4);

    let result: u32 = serializer.deserialize(&bytes).unwrap();
    assert_eq!(result, value);
}

#[test]
fn test_byte_serializer_roundtrip_u64() {
    let serializer = ByteSerializer::new();
    let value = 0x123456789ABCDEF0u64;

    let bytes = serializer.serialize(&value).unwrap();
    assert_eq!(bytes.len(), 8);

    let result: u64 = serializer.deserialize(&bytes).unwrap();
    assert_eq!(result, value);
}

#[test]
fn test_byte_serializer_roundtrip_bool() {
    let serializer = ByteSerializer::new();

    let bytes_true = serializer.serialize(&true).unwrap();
    let bytes_false = serializer.serialize(&false).unwrap();

    assert_eq!(serializer.deserialize::<bool>(&bytes_true).unwrap(), true);
    assert_eq!(serializer.deserialize::<bool>(&bytes_false).unwrap(), false);
}
