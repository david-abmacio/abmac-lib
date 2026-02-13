use bytecast::{FromBytes, ToBytesExt};

#[derive(Debug, PartialEq, bytecast::DeriveToBytes, bytecast::DeriveFromBytes)]
struct Point {
    x: u32,
    y: u32,
}

#[test]
fn test_derive_roundtrip() {
    let point = Point { x: 10, y: 20 };
    let bytes = point.to_vec().unwrap();
    let (decoded, _) = Point::from_bytes(&bytes).unwrap();
    assert_eq!(point, decoded);
}
