use alloc::vec::Vec;

use bytecast::{ByteCursor, ByteReader, BytesError, FromBytes, ToBytes};

use crate::Spout;

/// Prepends framing headers (producer_id, byte length, payload) before forwarding.
///
/// Each item is serialized via `ToBytes`, then wrapped in a frame:
/// `[producer_id: usize] [payload_len: u32 (4 bytes)] [payload bytes]`
///
/// Note: `usize` is serialized as `u64` on the wire by bytecast, so frames
/// are portable across architectures.
///
/// Compose with `ProducerSpout` for tagged, framed output.
pub struct FramedSpout<S> {
    inner: S,
    producer_id: usize,
    /// Reusable buffer to avoid per-send allocation.
    buf: Vec<u8>,
}

/// Fixed overhead per frame: serialized usize (always 8 bytes via bytecast) + u32 payload length.
const FRAME_HEADER_SIZE: usize = match (<usize as ToBytes>::MAX_SIZE, <u32 as ToBytes>::MAX_SIZE) {
    (Some(a), Some(b)) => a + b,
    _ => unreachable!(),
};

impl<S> FramedSpout<S> {
    /// Create a new framed spout.
    ///
    /// Each item sent will be serialized and wrapped in a frame with the
    /// given `producer_id` before forwarding to the inner spout.
    pub fn new(producer_id: usize, inner: S) -> Self {
        Self {
            inner,
            producer_id,
            buf: Vec::new(),
        }
    }

    /// Get the producer ID.
    pub fn producer_id(&self) -> usize {
        self.producer_id
    }

    /// Get a reference to the inner spout.
    pub fn inner(&self) -> &S {
        &self.inner
    }

    /// Get a mutable reference to the inner spout.
    pub fn inner_mut(&mut self) -> &mut S {
        &mut self.inner
    }

    /// Consume and return the inner spout.
    pub fn into_inner(self) -> S {
        self.inner
    }
}

impl<T: ToBytes, S: Spout<Vec<u8>, Error = core::convert::Infallible>> Spout<T> for FramedSpout<S> {
    type Error = BytesError;

    #[inline]
    fn send(&mut self, item: T) -> Result<(), Self::Error> {
        // Determine payload size
        let payload_size = item.byte_len().or(T::MAX_SIZE).unwrap_or(256);

        // Prepare buffer for header + payload
        self.buf.clear();
        self.buf.resize(FRAME_HEADER_SIZE + payload_size, 0);

        // Write payload first to learn actual size
        let payload_written = item.to_bytes(&mut self.buf[FRAME_HEADER_SIZE..])?;

        // Validate payload fits in u32 length field
        let payload_len =
            u32::try_from(payload_written).map_err(|_| BytesError::BufferTooSmall {
                needed: payload_written,
                available: u32::MAX as usize,
            })?;

        // Write header: producer_id + payload length
        let mut cursor = ByteCursor::new(&mut self.buf[..FRAME_HEADER_SIZE]);
        cursor.write(&self.producer_id)?;
        cursor.write(&payload_len)?;

        // Truncate to actual frame size and reuse buffer
        let total = FRAME_HEADER_SIZE + payload_written;
        self.buf.truncate(total);
        // Inner spout is infallible
        let _ = self.inner.send(self.buf.split_off(0));
        Ok(())
    }

    #[inline]
    fn flush(&mut self) -> Result<(), Self::Error> {
        let _ = self.inner.flush();
        Ok(())
    }
}

/// Decode a frame produced by `FramedSpout`.
///
/// Returns `(producer_id, item)` from the framed bytes. Validates that the
/// declared payload length matches the remaining frame bytes.
pub fn decode_frame<T: FromBytes>(frame: &[u8]) -> Result<(usize, T), BytesError> {
    let mut reader = ByteReader::new(frame);
    let producer_id: usize = reader.read()?;
    let payload_len: u32 = reader.read()?;
    let remaining = reader.remaining();
    if remaining.len() != payload_len as usize {
        return Err(BytesError::UnexpectedEof {
            needed: payload_len as usize,
            available: remaining.len(),
        });
    }
    let item: T = reader.read()?;
    let trailing = reader.remaining().len();
    if trailing != 0 {
        return Err(BytesError::InvalidData {
            message: "trailing bytes after payload in frame",
        });
    }
    Ok((producer_id, item))
}

// --- BatchSpout snapshot serialization ---

use crate::BatchSpout;

impl<T: ToBytes, S> ToBytes for BatchSpout<T, S> {
    const MAX_SIZE: Option<usize> = None;

    fn to_bytes(&self, buf: &mut [u8]) -> Result<usize, bytecast::BytesError> {
        let mut cursor = ByteCursor::new(buf);
        cursor.write(&self.threshold())?;
        cursor.write(&self.buffer)?;
        Ok(cursor.position())
    }

    fn byte_len(&self) -> Option<usize> {
        // usize threshold (8 bytes on wire via bytecast) + buffer byte_len
        Some(<usize as ToBytes>::MAX_SIZE? + self.buffer.byte_len()?)
    }
}

/// Decode a serialized `BatchSpout` snapshot.
///
/// Returns `(threshold, buffered_items)` from the bytes produced by
/// `BatchSpout::to_bytes()`. The caller reconstructs the `BatchSpout`
/// with their own sink and uses these values to restore state.
pub fn decode_batch<T: FromBytes>(bytes: &[u8]) -> Result<(usize, Vec<T>), BytesError> {
    let mut reader = ByteReader::new(bytes);
    let threshold: usize = reader.read()?;
    let buffer: Vec<T> = reader.read()?;
    let trailing = reader.remaining().len();
    if trailing != 0 {
        return Err(BytesError::InvalidData {
            message: "trailing bytes after batch payload",
        });
    }
    Ok((threshold, buffer))
}
