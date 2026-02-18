use alloc::string::String;
use alloc::vec::Vec;
use serde::de::Deserializer;
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

use crate::{FromBytes, ToBytes, ToBytesExt};

/// Wrapper providing serde support for any `ToBytes + FromBytes` type.
///
/// Serialization format depends on the serde format:
/// - Human-readable (JSON, TOML): bytes are base64-encoded as a string
/// - Binary (bincode, postcard): bytes are written as a raw byte slice
#[derive(Debug, Clone, PartialEq)]
pub struct BytecastSerde<T>(pub T);

impl<T> From<T> for BytecastSerde<T> {
    fn from(value: T) -> Self {
        BytecastSerde(value)
    }
}

impl<T> BytecastSerde<T> {
    /// Unwrap the inner value.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T: ToBytes> Serialize for BytecastSerde<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let bytes = self.0.to_vec().map_err(serde::ser::Error::custom)?;
        if serializer.is_human_readable() {
            use base64::Engine;
            let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
            serializer.serialize_str(&encoded)
        } else {
            serializer.serialize_bytes(&bytes)
        }
    }
}

impl<'de, T: FromBytes> Deserialize<'de> for BytecastSerde<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if deserializer.is_human_readable() {
            let s = String::deserialize(deserializer)?;
            use base64::Engine;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&s)
                .map_err(serde::de::Error::custom)?;
            let (val, _) = T::from_bytes(&bytes).map_err(serde::de::Error::custom)?;
            Ok(BytecastSerde(val))
        } else {
            let bytes = Vec::<u8>::deserialize(deserializer)?;
            let (val, _) = T::from_bytes(&bytes).map_err(serde::de::Error::custom)?;
            Ok(BytecastSerde(val))
        }
    }
}
