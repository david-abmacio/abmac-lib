use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, Hash};

use crate::{BytesError, FromBytes, ToBytes};

use super::alloc::{checked_len, var_int};

// HashSet<T> - same wire format as Vec<T>
impl<T: ToBytes, S> ToBytes for HashSet<T, S> {
    const MAX_SIZE: Option<usize> = None;

    fn to_bytes(&self, buf: &mut [u8]) -> Result<usize, BytesError> {
        let len = checked_len(self.len())?;
        let mut offset = var_int::encode(len, buf)?;
        for item in self {
            offset += item.to_bytes(&mut buf[offset..])?;
        }
        Ok(offset)
    }

    fn byte_len(&self) -> Option<usize> {
        let mut total = var_int::len(checked_len(self.len()).ok()?);
        for item in self {
            total += item.byte_len()?;
        }
        Some(total)
    }
}

impl<T: FromBytes + Eq + Hash, S: BuildHasher + Default> FromBytes for HashSet<T, S> {
    fn from_bytes(buf: &[u8]) -> Result<(Self, usize), BytesError> {
        let (len, mut offset) = var_int::decode(buf)?;
        let len = len as usize;
        let remaining = buf.len().saturating_sub(offset);
        if len > remaining {
            return Err(BytesError::UnexpectedEof {
                needed: offset + len,
                available: buf.len(),
            });
        }
        let mut set = HashSet::with_capacity_and_hasher(len, S::default());
        for _ in 0..len {
            let (item, n) = T::from_bytes(&buf[offset..])?;
            set.insert(item);
            offset += n;
        }
        Ok((set, offset))
    }
}

// HashMap<K, V> - varint length + interleaved key-value pairs
impl<K: ToBytes, V: ToBytes, S> ToBytes for HashMap<K, V, S> {
    const MAX_SIZE: Option<usize> = None;

    fn to_bytes(&self, buf: &mut [u8]) -> Result<usize, BytesError> {
        let len = checked_len(self.len())?;
        let mut offset = var_int::encode(len, buf)?;
        for (key, val) in self {
            offset += key.to_bytes(&mut buf[offset..])?;
            offset += val.to_bytes(&mut buf[offset..])?;
        }
        Ok(offset)
    }

    fn byte_len(&self) -> Option<usize> {
        let mut total = var_int::len(checked_len(self.len()).ok()?);
        for (key, val) in self {
            total += key.byte_len()?;
            total += val.byte_len()?;
        }
        Some(total)
    }
}

impl<K: FromBytes + Eq + Hash, V: FromBytes, S: BuildHasher + Default> FromBytes
    for HashMap<K, V, S>
{
    fn from_bytes(buf: &[u8]) -> Result<(Self, usize), BytesError> {
        let (len, mut offset) = var_int::decode(buf)?;
        let len = len as usize;
        let remaining = buf.len().saturating_sub(offset);
        if len > remaining {
            return Err(BytesError::UnexpectedEof {
                needed: offset + len,
                available: buf.len(),
            });
        }
        let mut map = HashMap::with_capacity_and_hasher(len, S::default());
        for _ in 0..len {
            let (key, kn) = K::from_bytes(&buf[offset..])?;
            offset += kn;
            let (val, vn) = V::from_bytes(&buf[offset..])?;
            offset += vn;
            map.insert(key, val);
        }
        Ok((map, offset))
    }
}
