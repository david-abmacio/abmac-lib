use bytecast::{DeriveFromBytes, DeriveToBytes, FromBytes, ToBytes};

#[derive(DeriveToBytes, DeriveFromBytes, Debug, PartialEq)]
struct Point {
    x: i32,
    y: i32,
}

fn main() {
    let p = Point { x: 10, y: 20 };
    let mut buf = [0u8; 8];
    p.to_bytes(&mut buf).unwrap();
    println!("serialized: {buf:?}");

    let (decoded, _) = Point::from_bytes(&buf).unwrap();
    println!("deserialized: {decoded:?}");

    assert_eq!(p, decoded);
}
