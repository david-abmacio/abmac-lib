//! Built-in serializer adapters.

/// Serializer for checkpoint types implementing bytecast's `ToBytes + FromBytes`.
///
/// This is the recommended serializer for most use cases. It produces compact
/// binary output with no external format dependencies. Users derive
/// `ToBytes + FromBytes` on their checkpoint type via `bytecast-macros`.
///
/// For external format transport (JSON, etc.), compose with bytecast's bridge
/// types (`BytecastSerde`, `BytecastFacet`) independently of pebble.
#[cfg(feature = "bytecast")]
#[derive(Clone, Copy)]
pub struct BytecastSerializer;

#[cfg(feature = "bytecast")]
impl<T: bytecast::ToBytes + bytecast::FromBytes> super::traits::CheckpointSerializer<T>
    for BytecastSerializer
{
    type Error = bytecast::BytesError;

    fn serialize(&self, value: &T) -> Result<alloc::vec::Vec<u8>, bytecast::BytesError> {
        bytecast::ToBytesExt::to_vec(value)
    }

    fn deserialize(&self, bytes: &[u8]) -> Result<T, bytecast::BytesError> {
        T::from_bytes(bytes).map(|(v, _)| v)
    }
}
