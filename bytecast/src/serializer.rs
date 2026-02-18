use alloc::vec::Vec;

use crate::{BytesError, FromBytes, ToBytes, traits::serialize_to_vec};

/// A serializer that uses ToBytes/FromBytes traits.
///
/// This provides a simple way to serialize types that implement the byte traits
/// into owned Vec<u8> buffers.
#[derive(Debug, Clone, Copy, Default)]
pub struct ByteSerializer;

impl ByteSerializer {
    /// Create a new ByteSerializer.
    pub const fn new() -> Self {
        Self
    }

    /// Serialize a value to bytes.
    pub fn serialize<T: ToBytes>(&self, value: &T) -> Result<Vec<u8>, BytesError> {
        let hint = value.byte_len().or(T::MAX_SIZE).unwrap_or(256);
        serialize_to_vec(value, hint)
    }

    /// Deserialize a value from bytes.
    pub fn deserialize<T: FromBytes>(&self, bytes: &[u8]) -> Result<T, BytesError> {
        let (value, _) = T::from_bytes(bytes)?;
        Ok(value)
    }
}

/// Write cursor for sequential serialization.
pub struct ByteCursor<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl<'a> ByteCursor<'a> {
    pub fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn write<T: ToBytes>(&mut self, value: &T) -> Result<usize, BytesError> {
        let n = value.to_bytes(&mut self.buf[self.pos..])?;
        self.pos += n;
        Ok(n)
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    pub fn written(&self) -> &[u8] {
        &self.buf[..self.pos]
    }
}

/// Read cursor for sequential deserialization.
pub struct ByteReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> ByteReader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn read<T: FromBytes>(&mut self) -> Result<T, BytesError> {
        let (v, n) = T::from_bytes(&self.buf[self.pos..])?;
        self.pos += n;
        Ok(v)
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> &[u8] {
        &self.buf[self.pos..]
    }
}
