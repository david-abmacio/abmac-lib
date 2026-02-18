//! Byte serialization traits and implementations.
//!
//! This crate provides `ToBytes` and `FromBytes` traits for serializing
//! Rust types to and from byte buffers. Fixed-size types are handled
//! via zerocopy (internal), while variable-length types like `Option<T>`,
//! `Vec<T>`, and `String` have native implementations.
#![no_std]

#[cfg(feature = "alloc")]
extern crate alloc;

mod error;
mod impls;
mod traits;

#[cfg(feature = "alloc")]
mod serializer;

pub use error::{BytesError, Result};
pub use impls::wrapper::ZeroCopyType;
pub use traits::{FromBytes, FromBytesExt, ToBytes, ToBytesExt};

// Re-export zerocopy derives for custom #[repr(C)] structs
pub use zerocopy::{FromBytes as ZcFromBytes, Immutable, IntoBytes, KnownLayout};

// Re-export derive macros when derive feature is enabled
#[cfg(feature = "derive")]
pub use abmac_macros::{FromBytes as DeriveFromBytes, ToBytes as DeriveToBytes};

#[cfg(feature = "alloc")]
pub use serializer::{ByteCursor, ByteReader, ByteSerializer};

#[cfg(any(feature = "serde", feature = "facet", feature = "rkyv"))]
pub mod bridges;

#[cfg(feature = "serde")]
pub use bridges::BytecastSerde;

#[cfg(feature = "facet")]
pub use bridges::BytecastFacet;

#[cfg(feature = "rkyv")]
pub use bridges::BytecastRkyv;

#[cfg(test)]
mod tests;
