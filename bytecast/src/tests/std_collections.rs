use std::collections::{HashMap, HashSet};

use super::{BytesError, FromBytes, ToBytes};

// --- HashSet tests ---

#[test]
fn test_hashset_roundtrip() {
    let original: HashSet<u32> = [3, 1, 4, 1, 5].into_iter().collect();
    let mut buf = [0u8; 64];
    let written = original.to_bytes(&mut buf).unwrap();
    let (decoded, consumed) = HashSet::<u32>::from_bytes(&buf).unwrap();
    assert_eq!(decoded, original);
    assert_eq!(consumed, written);
}

#[test]
fn test_hashset_empty() {
    let original: HashSet<u32> = HashSet::new();
    let mut buf = [0u8; 8];
    let written = original.to_bytes(&mut buf).unwrap();
    let (decoded, consumed) = HashSet::<u32>::from_bytes(&buf).unwrap();
    assert_eq!(decoded, original);
    assert_eq!(consumed, written);
    assert_eq!(written, 1); // varint 0
}

#[test]
fn test_hashset_byte_len() {
    let set: HashSet<u32> = [1, 2, 3].into_iter().collect();
    // varint(3) = 1 byte, 3 * 4 bytes = 12, total = 13
    assert_eq!(set.byte_len(), Some(13));
}

// --- HashMap tests ---

#[test]
fn test_hashmap_roundtrip() {
    let mut original = HashMap::new();
    original.insert(1u32, 10u64);
    original.insert(2, 20);
    original.insert(3, 30);
    let mut buf = [0u8; 128];
    let written = original.to_bytes(&mut buf).unwrap();
    let (decoded, consumed) = HashMap::<u32, u64>::from_bytes(&buf).unwrap();
    assert_eq!(decoded, original);
    assert_eq!(consumed, written);
}

#[test]
fn test_hashmap_empty() {
    let original: HashMap<u32, u32> = HashMap::new();
    let mut buf = [0u8; 8];
    let written = original.to_bytes(&mut buf).unwrap();
    let (decoded, consumed) = HashMap::<u32, u32>::from_bytes(&buf).unwrap();
    assert_eq!(decoded, original);
    assert_eq!(consumed, written);
    assert_eq!(written, 1); // varint 0
}

#[test]
fn test_hashmap_byte_len() {
    let mut map = HashMap::new();
    map.insert(1u32, 10u32);
    map.insert(2, 20);
    // varint(2) = 1 byte, 2 * (4 + 4) = 16, total = 17
    assert_eq!(map.byte_len(), Some(17));
}

#[test]
fn test_hashset_zst_rejects_oversized_length() {
    let buf = [0xFF, 0xFF, 0xFF, 0xFF, 0x0F];
    let result = HashSet::<()>::from_bytes(&buf);
    assert!(result.is_err());
}

#[test]
fn test_hashmap_zst_rejects_oversized_length() {
    let buf = [0xFF, 0xFF, 0xFF, 0xFF, 0x0F];
    let result = HashMap::<(), ()>::from_bytes(&buf);
    assert!(result.is_err());
}

#[test]
fn test_hashmap_string_keys() {
    use alloc::string::String;

    let mut original = HashMap::new();
    original.insert(String::from("hello"), 1u32);
    original.insert(String::from("world"), 2);
    let mut buf = [0u8; 128];
    let written = original.to_bytes(&mut buf).unwrap();
    let (decoded, consumed) = HashMap::<String, u32>::from_bytes(&buf).unwrap();
    assert_eq!(decoded, original);
    assert_eq!(consumed, written);
}

#[test]
fn test_hashset_truncated_varint() {
    // Empty buffer — can't even read the varint length
    let buf = [];
    let result = HashSet::<u32>::from_bytes(&buf);
    assert!(matches!(result, Err(BytesError::UnexpectedEof { .. })));
}
