# bytecast

Fast, simple byte serialization for Rust. Designed for embedded systems and performance-critical applications.

## Features

- **no_std by default** - Works on embedded devices with no allocator
- **Portable wire format** - Little-endian encoding for all fixed-size types
- **Simple API** - Just `ToBytes` and `FromBytes` traits
- **Variable-length encoding** - Efficient varint encoding for `Vec<T>` and `String` lengths

## Usage

__Note__: Bytecast is soon to be published soon. Please use the repository version until then.

Add to your `Cargo.toml`:

```toml
[dependencies]
bytecast = "1.0.0"
```

### Derive Macros

Use `DeriveToBytes` and `DeriveFromBytes` for structs and enums with variable-length or mixed fields. Supports `#[bytecast(skip)]` to exclude fields from serialization ([example](examples/derive.rs)).

### Sequential I/O

Use `ByteCursor` and `ByteReader` for writing and reading multiple values in sequence from a single buffer.

### Enums

Enums use a `u8` discriminant by default. Use `#[repr(uN)]` for larger discriminants. A compile-time error is emitted if the number of variants exceeds the discriminant capacity.

### Bridge Integrations

Optional bridges let bytecast-serialized data flow through other serialization frameworks. Enable the corresponding feature flag and see the matching example for usage:

- **serde** — `BytecastSerde<T>` ([example](examples/serde_bridge.rs))
- **facet** — `BytecastFacet` ([example](examples/facet_bridge.rs))
- **rkyv** — `BytecastRkyv` ([example](examples/rkyv_bridge.rs))

## Feature Flags

| Feature  | Description |
|----------|-------------|
| `alloc`  | Enables `Vec<T>`, `String`, `VecDeque<T>`, and `Cow` support |
| `std`    | Enables `HashMap`, `HashSet` (implies `alloc`) |
| `derive` | Enables `DeriveToBytes` and `DeriveFromBytes` proc macros |
| `serde`  | Enables `BytecastSerde<T>` bridge (implies `alloc`) |
| `facet`  | Enables `BytecastFacet` bridge (implies `alloc`) |
| `rkyv`   | Enables `BytecastRkyv` bridge (implies `alloc`) |

## Supported Types

| Type | Encoding | Feature |
|------|----------|---------|
| `u8` | 1 byte | - |
| `u16` | 2 bytes | - |
| `u32` | 4 bytes | - |
| `u64` | 8 bytes | - |
| `u128` | 16 bytes | - |
| `i8` | 1 byte | - |
| `i16` | 2 bytes | - |
| `i32` | 4 bytes | - |
| `i64` | 8 bytes | - |
| `i128` | 16 bytes | - |
| `f32` | 4 bytes | - |
| `f64` | 8 bytes | - |
| `usize` | 8 bytes (as `u64`) | - |
| `isize` | 8 bytes (as `i64`) | - |
| `bool` | 1 byte (validated) | - |
| `char` | 4 bytes (validated) | - |
| `()` | 0 bytes | - |
| `[T; N]` | `N * size_of::<T>()` | - |
| `Option<T>` | 1 byte discriminant + payload | - |
| `Result<T, E>` | 1 byte discriminant + payload | - |
| `(A, B, ...)` | Elements in order (arity 1-12) | - |
| `Range<T>` | start + end | - |
| `RangeInclusive<T>` | start + end | - |
| `NonZeroU8/16/32/64/128` | Same as inner type (validated) | - |
| `NonZeroI8/16/32/64/128` | Same as inner type (validated) | - |
| `#[repr(C)]` structs | Size of struct | - |
| `Vec<T>` | Varint length + elements | `alloc` |
| `VecDeque<T>` | Varint length + elements | `alloc` |
| `String` | Varint length + UTF-8 bytes | `alloc` |
| `Cow<'_, str>` | Varint length + UTF-8 bytes | `alloc` |
| `Cow<'_, [T]>` | Varint length + elements | `alloc` |
| `BTreeMap<K, V>` | Varint length + key-value pairs | `alloc` |
| `BTreeSet<T>` | Varint length + elements | `alloc` |
| `HashMap<K, V>` | Varint length + key-value pairs | `std` |
| `HashSet<T>` | Varint length + elements | `std` |

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.
