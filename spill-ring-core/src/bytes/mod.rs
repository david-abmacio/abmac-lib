//! Byte serialization traits and implementations.

mod error;
mod impls;
mod serializer;
mod traits;

pub use error::{BytesError, Result};
pub use serializer::ByteSerializer;
pub use traits::{FromBytes, ToBytes, ViewBytes};

#[cfg(test)]
mod tests;
