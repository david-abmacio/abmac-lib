//! Integration tests for derive macros.

use bytecast::{BytesError, DeriveFromBytes, DeriveToBytes, FromBytes, ToBytes};

#[derive(DeriveToBytes, DeriveFromBytes, Debug, PartialEq)]
struct UnitStruct;

#[test]
fn test_derive_unit_struct() {
    let mut buf = [0u8; 8];
    let value = UnitStruct;

    let written = value.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 0);
    assert_eq!(UnitStruct::MAX_SIZE, Some(0));

    let (decoded, consumed) = UnitStruct::from_bytes(&buf).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(consumed, 0);
}

#[derive(DeriveToBytes, DeriveFromBytes, Debug, PartialEq)]
struct SimpleStruct {
    a: u32,
    b: u16,
}

#[test]
fn test_derive_simple_struct() {
    let mut buf = [0u8; 16];
    let value = SimpleStruct {
        a: 0x12345678,
        b: 0xABCD,
    };

    let written = value.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 6); // 4 + 2
    assert_eq!(SimpleStruct::MAX_SIZE, Some(6));

    let (decoded, consumed) = SimpleStruct::from_bytes(&buf).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(consumed, 6);
}

#[derive(DeriveToBytes, DeriveFromBytes, Debug, PartialEq)]
struct TupleStruct(u32, u8);

#[test]
fn test_derive_tuple_struct() {
    let mut buf = [0u8; 16];
    let value = TupleStruct(42, 7);

    let written = value.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 5); // 4 + 1
    assert_eq!(TupleStruct::MAX_SIZE, Some(5));

    let (decoded, consumed) = TupleStruct::from_bytes(&buf).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(consumed, 5);
}

#[derive(DeriveToBytes, DeriveFromBytes, Debug, PartialEq)]
struct NestedStruct {
    inner: SimpleStruct,
    flag: bool,
}

#[test]
fn test_derive_nested_struct() {
    let mut buf = [0u8; 16];
    let value = NestedStruct {
        inner: SimpleStruct { a: 100, b: 200 },
        flag: true,
    };

    let written = value.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 7); // 6 + 1
    assert_eq!(NestedStruct::MAX_SIZE, Some(7));

    let (decoded, consumed) = NestedStruct::from_bytes(&buf).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(consumed, 7);
}

#[derive(DeriveToBytes, DeriveFromBytes, Debug, PartialEq)]
enum UnitEnum {
    A,
    B,
    C,
}

#[test]
fn test_derive_unit_enum() {
    let mut buf = [0u8; 8];

    for (i, value) in [UnitEnum::A, UnitEnum::B, UnitEnum::C].iter().enumerate() {
        let written = value.to_bytes(&mut buf).unwrap();
        assert_eq!(written, 1);
        assert_eq!(buf[0], i as u8);

        let (decoded, consumed) = UnitEnum::from_bytes(&buf).unwrap();
        assert_eq!(&decoded, value);
        assert_eq!(consumed, 1);
    }

    assert_eq!(UnitEnum::MAX_SIZE, Some(1));
}

#[derive(DeriveToBytes, DeriveFromBytes, Debug, PartialEq)]
enum TupleEnum {
    Empty,
    Single(u32),
    Double(u16, u8),
}

#[test]
fn test_derive_tuple_enum() {
    let mut buf = [0u8; 16];

    // Empty variant
    let value = TupleEnum::Empty;
    let written = value.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 1);
    let (decoded, _) = TupleEnum::from_bytes(&buf).unwrap();
    assert_eq!(decoded, value);

    // Single variant
    let value = TupleEnum::Single(0x12345678);
    let written = value.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 5); // 1 + 4
    let (decoded, _) = TupleEnum::from_bytes(&buf).unwrap();
    assert_eq!(decoded, value);

    // Double variant
    let value = TupleEnum::Double(0xABCD, 0x42);
    let written = value.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 4); // 1 + 2 + 1
    let (decoded, _) = TupleEnum::from_bytes(&buf).unwrap();
    assert_eq!(decoded, value);

    // MAX_SIZE should be 1 (discriminant) + 4 (largest variant = Single)
    assert_eq!(TupleEnum::MAX_SIZE, Some(5));
}

#[derive(DeriveToBytes, DeriveFromBytes, Debug, PartialEq)]
enum StructEnum {
    Empty,
    Point { x: i32, y: i32 },
    Named { id: u8, value: u64 },
}

#[test]
fn test_derive_struct_enum() {
    let mut buf = [0u8; 32];

    // Empty variant
    let value = StructEnum::Empty;
    let written = value.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 1);
    let (decoded, _) = StructEnum::from_bytes(&buf).unwrap();
    assert_eq!(decoded, value);

    // Point variant
    let value = StructEnum::Point { x: -10, y: 20 };
    let written = value.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 9); // 1 + 4 + 4
    let (decoded, _) = StructEnum::from_bytes(&buf).unwrap();
    assert_eq!(decoded, value);

    // Named variant
    let value = StructEnum::Named {
        id: 42,
        value: 0x123456789ABCDEF0,
    };
    let written = value.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 10); // 1 + 1 + 8
    let (decoded, _) = StructEnum::from_bytes(&buf).unwrap();
    assert_eq!(decoded, value);

    // MAX_SIZE should be 1 + 9 (Named is largest: 1 + 8)
    assert_eq!(StructEnum::MAX_SIZE, Some(10));
}

#[test]
fn test_derive_enum_invalid_discriminant() {
    let buf = [255u8]; // Invalid discriminant for UnitEnum
    let result = UnitEnum::from_bytes(&buf);
    assert!(matches!(result, Err(BytesError::InvalidData { .. })));
}

#[derive(DeriveToBytes, DeriveFromBytes, Debug, PartialEq)]
struct SkipNamed {
    a: u32,
    #[bytecast(skip)]
    b: u16,
    c: u8,
}

#[test]
fn test_derive_skip_named_field() {
    let mut buf = [0u8; 16];
    let value = SkipNamed {
        a: 0x12345678,
        b: 0xABCD,
        c: 42,
    };

    // b is skipped, so only a (4) + c (1) = 5 bytes
    let written = value.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 5);
    assert_eq!(SkipNamed::MAX_SIZE, Some(5));
    assert_eq!(value.byte_len(), Some(5));

    // Deserialize: b should be Default (0)
    let (decoded, consumed) = SkipNamed::from_bytes(&buf).unwrap();
    assert_eq!(consumed, 5);
    assert_eq!(decoded.a, 0x12345678);
    assert_eq!(decoded.b, 0); // default
    assert_eq!(decoded.c, 42);
}

#[derive(DeriveToBytes, DeriveFromBytes, Debug, PartialEq)]
struct SkipTuple(u32, #[bytecast(skip)] u16, u8);

#[test]
fn test_derive_skip_tuple_field() {
    let mut buf = [0u8; 16];
    let value = SkipTuple(0x12345678, 0xABCD, 42);

    let written = value.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 5); // 4 + 1, skipping the u16
    assert_eq!(SkipTuple::MAX_SIZE, Some(5));

    let (decoded, consumed) = SkipTuple::from_bytes(&buf).unwrap();
    assert_eq!(consumed, 5);
    assert_eq!(decoded.0, 0x12345678);
    assert_eq!(decoded.1, 0); // default
    assert_eq!(decoded.2, 42);
}

#[derive(DeriveToBytes, DeriveFromBytes, Debug, PartialEq)]
#[repr(u32)]
enum ReprU32Enum {
    A,
    B,
    C(u8),
}

#[test]
fn test_derive_repr_u32_unit_variant() {
    let mut buf = [0u8; 16];

    let value = ReprU32Enum::A;
    let written = value.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 4); // u32 discriminant
    assert_eq!(value.byte_len(), Some(4));

    let (decoded, consumed) = ReprU32Enum::from_bytes(&buf).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(consumed, 4);
}

#[test]
fn test_derive_repr_u32_data_variant() {
    let mut buf = [0u8; 16];

    let value = ReprU32Enum::C(42);
    let written = value.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 5); // 4 byte discriminant + 1 byte u8
    assert_eq!(value.byte_len(), Some(5));

    let (decoded, consumed) = ReprU32Enum::from_bytes(&buf).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(consumed, 5);
}

#[test]
fn test_derive_repr_u32_max_size() {
    // MAX_SIZE = size_of::<u32>() + max variant payload (1 byte for C)
    assert_eq!(ReprU32Enum::MAX_SIZE, Some(5));
}

#[test]
fn test_derive_repr_u32_invalid_discriminant() {
    // Write a u32 discriminant of 99 — should fail
    let mut buf = [0u8; 4];
    let _ = 99u32.to_bytes(&mut buf).unwrap();
    let result = ReprU32Enum::from_bytes(&buf);
    assert!(matches!(result, Err(BytesError::InvalidData { .. })));
}

#[derive(DeriveToBytes, DeriveFromBytes, Debug, PartialEq)]
#[repr(u16)]
enum ReprU16Enum {
    X,
    Y { value: u32 },
}

#[test]
fn test_derive_repr_u16_enum() {
    let mut buf = [0u8; 16];

    let value = ReprU16Enum::Y { value: 0xDEADBEEF };
    let written = value.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 6); // 2 byte discriminant + 4 byte u32

    let (decoded, consumed) = ReprU16Enum::from_bytes(&buf).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(consumed, 6);

    assert_eq!(ReprU16Enum::MAX_SIZE, Some(6));
}

#[derive(DeriveToBytes, DeriveFromBytes, Debug, PartialEq)]
struct BoxedNamed {
    id: u16,
    #[bytecast(boxed)]
    value: Box<u32>,
}

#[derive(DeriveToBytes, DeriveFromBytes, Debug, PartialEq)]
struct BoxedTuple(u8, #[bytecast(boxed)] Box<u32>);

#[test]
fn test_boxed_named_roundtrip() {
    let original = BoxedNamed {
        id: 1,
        value: Box::new(0x12345678),
    };
    let mut buf = [0u8; 64];
    let written = original.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 6); // 2 + 4

    let (decoded, consumed) = BoxedNamed::from_bytes(&buf).unwrap();
    assert_eq!(consumed, 6);
    assert_eq!(decoded, original);
}

#[test]
fn test_boxed_tuple_roundtrip() {
    let original = BoxedTuple(42, Box::new(0xDEADBEEF));
    let mut buf = [0u8; 64];
    let written = original.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 5); // 1 + 4

    let (decoded, consumed) = BoxedTuple::from_bytes(&buf).unwrap();
    assert_eq!(consumed, 5);
    assert_eq!(decoded, original);
}

#[test]
fn test_boxed_max_size() {
    assert_eq!(BoxedNamed::MAX_SIZE, Some(6));
    assert_eq!(BoxedTuple::MAX_SIZE, Some(5));
}

use core::marker::PhantomData;

#[derive(DeriveToBytes, DeriveFromBytes, Debug, PartialEq)]
struct WithPhantom<T> {
    id: u32,
    _marker: PhantomData<T>,
}

#[test]
fn test_phantom_data_auto_skip() {
    let original = WithPhantom::<String> {
        id: 42,
        _marker: PhantomData,
    };
    let mut buf = [0u8; 4];
    let written = original.to_bytes(&mut buf).unwrap();
    assert_eq!(written, 4);

    let (decoded, consumed) = WithPhantom::<String>::from_bytes(&buf).unwrap();
    assert_eq!(consumed, 4);
    assert_eq!(decoded, original);
    assert_eq!(WithPhantom::<String>::MAX_SIZE, Some(4));
}

#[test]
fn test_derive_byte_len() {
    let s = SimpleStruct { a: 1, b: 2 };
    assert_eq!(s.byte_len(), Some(6));

    let e = TupleEnum::Single(42);
    assert_eq!(e.byte_len(), Some(5));

    let e = TupleEnum::Empty;
    assert_eq!(e.byte_len(), Some(1));
}
