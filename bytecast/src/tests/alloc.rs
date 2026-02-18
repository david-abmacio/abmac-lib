use alloc::{borrow::Cow, string::String, vec, vec::Vec};

use super::{
    ByteCursor, ByteReader, ByteSerializer, BytesError, FromBytes, FromBytesExt, ToBytes,
    ToBytesExt, ZeroCopyType,
};
use zerocopy::{FromBytes as ZcFromBytes, Immutable, IntoBytes, KnownLayout};

#[test]
fn test_byte_serializer_new() {
    let _serializer = ByteSerializer::new();
    let _default = ByteSerializer;
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

    assert!(serializer.deserialize::<bool>(&bytes_true).unwrap());
    assert!(!serializer.deserialize::<bool>(&bytes_false).unwrap());
}

#[test]
fn test_vec_u8_roundtrip() {
    let original: Vec<u8> = vec![1, 2, 3, 4, 5];
    let mut buf = [0u8; 64];

    let written = original.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 1 + 5); // 1 byte var_int length + 5 bytes data

    let (decoded, consumed) = Vec::<u8>::from_bytes(&buf).unwrap();
    assert_eq!(decoded, original);
    assert_eq!(consumed, written);
}

#[test]
fn test_vec_u32_roundtrip() {
    let original: Vec<u32> = vec![100, 200, 300];
    let mut buf = [0u8; 64];

    let written = original.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 1 + 12); // 1 byte var_int length + 3*4 bytes data

    let (decoded, consumed) = Vec::<u32>::from_bytes(&buf).unwrap();
    assert_eq!(decoded, original);
    assert_eq!(consumed, written);
}

#[test]
fn test_vec_empty() {
    let original: Vec<u8> = vec![];
    let mut buf = [0u8; 64];

    let written = original.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 1); // just the length prefix

    let (decoded, consumed) = Vec::<u8>::from_bytes(&buf).unwrap();
    assert_eq!(decoded, original);
    assert_eq!(consumed, written);
}

#[test]
fn test_vec_large_length_prefix() {
    // 128 elements requires 2-byte var_int
    let original: Vec<u8> = vec![0u8; 128];
    let mut buf = [0u8; 256];

    let written = original.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 2 + 128); // 2 byte var_int length + 128 bytes data

    let (decoded, consumed) = Vec::<u8>::from_bytes(&buf).unwrap();
    assert_eq!(decoded, original);
    assert_eq!(consumed, written);
}

#[test]
fn test_string_roundtrip() {
    let original = String::from("hello world");
    let mut buf = [0u8; 64];

    let written = original.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 1 + 11); // 1 byte var_int length + 11 bytes

    let (decoded, consumed) = String::from_bytes(&buf).unwrap();
    assert_eq!(decoded, original);
    assert_eq!(consumed, written);
}

#[test]
fn test_string_empty() {
    let original = String::new();
    let mut buf = [0u8; 64];

    let written = original.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 1); // just the length prefix

    let (decoded, consumed) = String::from_bytes(&buf).unwrap();
    assert_eq!(decoded, original);
    assert_eq!(consumed, written);
}

#[test]
fn test_string_unicode() {
    let original = String::from("héllo 世界 🦀");
    let mut buf = [0u8; 64];

    let written = original.to_bytes(&mut buf).unwrap();

    let (decoded, consumed) = String::from_bytes(&buf).unwrap();
    assert_eq!(decoded, original);
    assert_eq!(consumed, written);
}

// Test zerocopy-derived struct via blanket impl
#[derive(ZcFromBytes, IntoBytes, Immutable, KnownLayout, Debug, PartialEq)]
#[repr(C)]
struct Point {
    x: i32,
    y: i32,
}

impl ZeroCopyType for Point {}

#[test]
fn test_zerocopy_struct_roundtrip() {
    let mut buf = [0u8; 8];
    let value = Point { x: 100, y: -200 };

    let written = value.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 8);

    let (decoded, consumed) = Point::from_bytes(&buf).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(consumed, 8);
}

// ByteCursor tests
#[test]
fn test_byte_cursor_write_multiple() {
    let mut buf = [0u8; 16];
    let mut cursor = ByteCursor::new(&mut buf);

    assert_eq!(cursor.position(), 0);
    assert_eq!(cursor.remaining(), 16);

    cursor.write(&42u32).unwrap();
    assert_eq!(cursor.position(), 4);
    assert_eq!(cursor.remaining(), 12);

    cursor.write(&100u64).unwrap();
    assert_eq!(cursor.position(), 12);
    assert_eq!(cursor.remaining(), 4);

    let written = cursor.written();
    assert_eq!(written.len(), 12);
}

#[test]
fn test_byte_cursor_sequential_values() {
    let mut buf = [0u8; 32];
    let mut cursor = ByteCursor::new(&mut buf);

    cursor.write(&1u8).unwrap();
    cursor.write(&2u16).unwrap();
    cursor.write(&3u32).unwrap();
    cursor.write(&4u64).unwrap();

    assert_eq!(cursor.position(), 1 + 2 + 4 + 8);
}

// ByteReader tests
#[test]
fn test_byte_reader_read_multiple() {
    let mut buf = [0u8; 16];
    let mut cursor = ByteCursor::new(&mut buf);
    cursor.write(&42u32).unwrap();
    cursor.write(&100u64).unwrap();

    let mut reader = ByteReader::new(cursor.written());
    assert_eq!(reader.position(), 0);
    assert_eq!(reader.remaining().len(), 12);

    let v1: u32 = reader.read().unwrap();
    assert_eq!(v1, 42);
    assert_eq!(reader.position(), 4);

    let v2: u64 = reader.read().unwrap();
    assert_eq!(v2, 100);
    assert_eq!(reader.position(), 12);
    assert_eq!(reader.remaining().len(), 0);
}

#[test]
fn test_cursor_reader_roundtrip_vec() {
    let mut buf = [0u8; 64];
    let mut cursor = ByteCursor::new(&mut buf);

    let original: Vec<u32> = vec![10, 20, 30, 40];
    cursor.write(&original).unwrap();

    let mut reader = ByteReader::new(cursor.written());
    let decoded: Vec<u32> = reader.read().unwrap();
    assert_eq!(decoded, original);
}

#[test]
fn test_cursor_reader_roundtrip_string() {
    let mut buf = [0u8; 64];
    let mut cursor = ByteCursor::new(&mut buf);

    let original = String::from("hello cursor");
    cursor.write(&original).unwrap();

    let mut reader = ByteReader::new(cursor.written());
    let decoded: String = reader.read().unwrap();
    assert_eq!(decoded, original);
}

#[test]
fn test_byte_serializer_vec() {
    let serializer = ByteSerializer::new();
    let original: Vec<u8> = vec![1, 2, 3, 4, 5];

    let bytes = serializer.serialize(&original).unwrap();
    let decoded: Vec<u8> = serializer.deserialize(&bytes).unwrap();
    assert_eq!(decoded, original);
}

#[test]
fn test_byte_serializer_string() {
    let serializer = ByteSerializer::new();
    let original = String::from("test string");

    let bytes = serializer.serialize(&original).unwrap();
    let decoded: String = serializer.deserialize(&bytes).unwrap();
    assert_eq!(decoded, original);
}

// ToBytesExt / FromBytesExt tests
#[test]
fn test_to_vec() {
    let value = 0x12345678u32;
    let bytes = value.to_vec().unwrap();
    assert_eq!(bytes.len(), 4);

    let (decoded, _) = u32::from_bytes(&bytes).unwrap();
    assert_eq!(decoded, value);
}

#[test]
fn test_to_array() {
    let value = 0x12345678u32;
    let arr: [u8; 4] = value.to_array().unwrap();

    let (decoded, _) = u32::from_bytes(&arr).unwrap();
    assert_eq!(decoded, value);
}

#[test]
fn test_to_array_size_mismatch() {
    let value = 0x12345678u32;
    let result: Result<[u8; 8], _> = value.to_array(); // u32 is 4 bytes, not 8
    assert!(result.is_err());
}

#[test]
fn test_from_bytes_partial() {
    let buf = [0x78, 0x56, 0x34, 0x12, 0xFF, 0xFF]; // u32 + trailing bytes
    let value = u32::from_bytes_partial(&buf).unwrap();
    assert_eq!(value, 0x12345678);
}

#[test]
fn test_from_bytes_exact() {
    let buf = [0x78, 0x56, 0x34, 0x12];
    let value = u32::from_bytes_exact(&buf).unwrap();
    assert_eq!(value, 0x12345678);
}

#[test]
fn test_from_bytes_exact_trailing() {
    let buf = [0x78, 0x56, 0x34, 0x12, 0xFF]; // trailing byte
    let result = u32::from_bytes_exact(&buf);
    assert!(result.is_err());
}

// Cow tests
#[test]
fn test_cow_str_borrowed_roundtrip() {
    let original: Cow<'_, str> = Cow::Borrowed("hello");
    let mut buf = [0u8; 64];

    let written = original.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 1 + 5); // varint len + bytes

    let (decoded, consumed) = Cow::<'_, str>::from_bytes(&buf).unwrap();
    assert_eq!(decoded, "hello");
    assert_eq!(consumed, written);
}

#[test]
fn test_cow_str_owned_roundtrip() {
    let original: Cow<'_, str> = Cow::Owned(String::from("world"));
    let mut buf = [0u8; 64];

    let written = original.to_bytes(&mut buf).unwrap();
    let (decoded, consumed) = Cow::<'_, str>::from_bytes(&buf).unwrap();
    assert_eq!(decoded, "world");
    assert_eq!(consumed, written);
}

#[test]
fn test_cow_str_wire_compat_with_string() {
    // Cow<str> and String should produce identical bytes
    let s = String::from("compatible");
    let c: Cow<'_, str> = Cow::Borrowed("compatible");

    let mut buf_s = [0u8; 64];
    let mut buf_c = [0u8; 64];
    let n_s = s.to_bytes(&mut buf_s).unwrap();
    let n_c = c.to_bytes(&mut buf_c).unwrap();

    assert_eq!(n_s, n_c);
    assert_eq!(&buf_s[..n_s], &buf_c[..n_c]);
}

#[test]
fn test_cow_slice_roundtrip() {
    let original: Cow<'_, [u32]> = Cow::Owned(vec![10, 20, 30]);
    let mut buf = [0u8; 64];

    let written = original.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 1 + 12); // varint len + 3*4

    let (decoded, consumed) = Cow::<'_, [u32]>::from_bytes(&buf).unwrap();
    assert_eq!(&*decoded, &[10, 20, 30]);
    assert_eq!(consumed, written);
}

#[test]
fn test_cow_slice_wire_compat_with_vec() {
    let v: Vec<u32> = vec![1, 2, 3];
    let c: Cow<'_, [u32]> = Cow::Owned(vec![1, 2, 3]);

    let mut buf_v = [0u8; 64];
    let mut buf_c = [0u8; 64];
    let n_v = v.to_bytes(&mut buf_v).unwrap();
    let n_c = c.to_bytes(&mut buf_c).unwrap();

    assert_eq!(n_v, n_c);
    assert_eq!(&buf_v[..n_v], &buf_c[..n_c]);
}

// VecDeque tests
#[test]
fn test_vecdeque_roundtrip() {
    use alloc::collections::VecDeque;

    let original: VecDeque<u32> = VecDeque::from([10, 20, 30]);
    let mut buf = [0u8; 64];

    let written = original.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 1 + 12); // varint len + 3*4

    let (decoded, consumed) = VecDeque::<u32>::from_bytes(&buf).unwrap();
    assert_eq!(decoded, original);
    assert_eq!(consumed, written);
}

#[test]
fn test_vecdeque_empty() {
    use alloc::collections::VecDeque;

    let original: VecDeque<u8> = VecDeque::new();
    let mut buf = [0u8; 64];

    let written = original.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 1);

    let (decoded, consumed) = VecDeque::<u8>::from_bytes(&buf).unwrap();
    assert_eq!(decoded, original);
    assert_eq!(consumed, written);
}

#[test]
fn test_vecdeque_wire_compat_with_vec() {
    use alloc::collections::VecDeque;

    let v: Vec<u32> = vec![1, 2, 3];
    let d: VecDeque<u32> = VecDeque::from([1, 2, 3]);

    let mut buf_v = [0u8; 64];
    let mut buf_d = [0u8; 64];
    let n_v = v.to_bytes(&mut buf_v).unwrap();
    let n_d = d.to_bytes(&mut buf_d).unwrap();

    assert_eq!(n_v, n_d);
    assert_eq!(&buf_v[..n_v], &buf_d[..n_d]);
}

#[test]
fn test_tuple_mixed() {
    let value: (u32, String) = (42, String::from("hello"));

    let bytes = ToBytesExt::to_vec(&value).unwrap();
    let (decoded, _) = <(u32, String)>::from_bytes(&bytes).unwrap();
    assert_eq!(decoded, value);

    assert_eq!(<(u32, String)>::MAX_SIZE, None);
}

#[test]
fn test_varint_rejects_5th_byte_overflow() {
    // 5th byte = 0x1F — bits 4-6 would overflow u32, must be rejected.
    let buf = [0xFF, 0xFF, 0xFF, 0xFF, 0x1F];
    let result = Vec::<u8>::from_bytes(&buf);
    assert!(result.is_err());
}

#[test]
fn test_varint_accepts_5th_byte_within_range() {
    // 5th byte = 0x0F is valid (uses exactly 4 bits for u32).
    // Encodes u32::MAX = [0xFF, 0xFF, 0xFF, 0xFF, 0x0F].
    // This claims u32::MAX elements so from_bytes will fail on length,
    // but NOT on the varint decode itself. Verify we get UnexpectedEof, not InvalidData.
    let buf = [0xFF, 0xFF, 0xFF, 0xFF, 0x0F];
    let result = Vec::<u8>::from_bytes(&buf);
    assert!(matches!(result, Err(BytesError::UnexpectedEof { .. })));
}

#[test]
fn test_varint_rejects_non_canonical() {
    // Value 1 encoded as 2 bytes: [0x81, 0x00] instead of canonical [0x01]
    let buf = [0x81, 0x00, 0x01]; // non-canonical length 1, then one byte of data
    let result = Vec::<u8>::from_bytes(&buf);
    assert!(result.is_err());
}

#[test]
fn test_vec_zst_rejects_oversized_length() {
    // Craft a payload claiming u32::MAX elements of () (ZST).
    // varint encoding of u32::MAX = [0xFF, 0xFF, 0xFF, 0xFF, 0x0F]
    let buf = [0xFF, 0xFF, 0xFF, 0xFF, 0x0F];
    let result = Vec::<()>::from_bytes(&buf);
    assert!(result.is_err());
}

#[test]
fn test_vec_zst_valid_small() {
    // A valid Vec<()> with 3 elements — length fits in remaining buffer.
    let mut buf = [0u8; 64];
    let original: Vec<()> = vec![(), (), ()];
    let written = original.to_bytes(&mut buf).unwrap();
    let (decoded, consumed) = Vec::<()>::from_bytes(&buf).unwrap();
    assert_eq!(decoded, original);
    assert_eq!(consumed, written);
}

#[test]
fn test_vecdeque_zst_rejects_oversized_length() {
    use alloc::collections::VecDeque;
    let buf = [0xFF, 0xFF, 0xFF, 0xFF, 0x0F];
    let result = VecDeque::<()>::from_bytes(&buf);
    assert!(result.is_err());
}
