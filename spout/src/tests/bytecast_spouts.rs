extern crate std;

use std::vec;
use std::vec::Vec;

use bytecast::{FromBytes, ToBytesExt};

use crate::{BatchSpout, CollectSpout, FramedSpout, Spout, decode_batch, decode_frame};

// --- FramedSpout tests ---

#[test]
fn framed_spout_sends_framed_bytes() {
    let mut s = FramedSpout::new(7, CollectSpout::<Vec<u8>>::new());

    let _ = s.send(42u32);

    let frames = s.inner().items();
    assert_eq!(frames.len(), 1);

    let (producer_id, value) = decode_frame::<u32>(&frames[0]).unwrap();
    assert_eq!(producer_id, 7);
    assert_eq!(value, 42);
}

#[test]
fn framed_spout_multiple_items() {
    let mut s = FramedSpout::new(0, CollectSpout::<Vec<u8>>::new());

    let _ = s.send(10u32);
    let _ = s.send(20u32);
    let _ = s.send(30u32);

    let frames = s.inner().items();
    assert_eq!(frames.len(), 3);

    let (id0, v0) = decode_frame::<u32>(&frames[0]).unwrap();
    let (id1, v1) = decode_frame::<u32>(&frames[1]).unwrap();
    let (id2, v2) = decode_frame::<u32>(&frames[2]).unwrap();

    assert_eq!((id0, v0), (0, 10));
    assert_eq!((id1, v1), (0, 20));
    assert_eq!((id2, v2), (0, 30));
}

#[test]
fn framed_spout_flush_delegates() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static FLUSH_COUNT: AtomicUsize = AtomicUsize::new(0);

    struct FlushTracker;
    impl Spout<Vec<u8>> for FlushTracker {
        type Error = core::convert::Infallible;
        fn send(&mut self, _item: Vec<u8>) -> Result<(), Self::Error> {
            Ok(())
        }
        fn flush(&mut self) -> Result<(), Self::Error> {
            FLUSH_COUNT.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    FLUSH_COUNT.store(0, Ordering::SeqCst);

    let mut s = FramedSpout::new(0, FlushTracker);
    let _ = <FramedSpout<FlushTracker> as Spout<u32>>::flush(&mut s);
    assert_eq!(FLUSH_COUNT.load(Ordering::SeqCst), 1);
}

#[test]
fn framed_spout_accessors() {
    let s = FramedSpout::new(42, CollectSpout::<Vec<u8>>::new());
    assert_eq!(s.producer_id(), 42);
    let _inner = s.inner();
}

#[test]
fn framed_spout_into_inner() {
    let mut s = FramedSpout::new(0, CollectSpout::<Vec<u8>>::new());
    let _ = s.send(1u32);

    let inner = s.into_inner();
    assert_eq!(inner.items().len(), 1);
}

// --- BatchSpout ToBytes tests ---

#[test]
fn batch_spout_to_bytes_empty_buffer() {
    let s: BatchSpout<u32, CollectSpout<Vec<u32>>> = BatchSpout::new(10, CollectSpout::new());

    let bytes = s.to_vec().unwrap();

    // Should serialize threshold (8 bytes on wire via bytecast) + empty vec
    assert!(!bytes.is_empty());

    // Verify threshold round-trips
    let (threshold, offset) = usize::from_bytes(&bytes).unwrap();
    assert_eq!(threshold, 10);
    assert!(offset > 0);
}

#[test]
fn batch_spout_to_bytes_with_buffered_items() {
    let mut s: BatchSpout<u32, CollectSpout<Vec<u32>>> = BatchSpout::new(100, CollectSpout::new());

    let _ = s.send(1);
    let _ = s.send(2);
    let _ = s.send(3);

    let bytes = s.to_vec().unwrap();

    // Decode: threshold then buffer
    let (threshold, offset) = usize::from_bytes(&bytes).unwrap();
    assert_eq!(threshold, 100);

    let (buffer, _) = Vec::<u32>::from_bytes(&bytes[offset..]).unwrap();
    assert_eq!(buffer, vec![1, 2, 3]);
}

// --- Composition pattern tests ---

#[test]
fn fn_spout_with_to_bytes_serialization() {
    use crate::FnSpout;

    let mut serialized = Vec::new();
    {
        let mut s = FnSpout::new(|item: u32| {
            serialized.extend(item.to_vec().unwrap());
        });
        let _ = s.send(1u32);
        let _ = s.send(2u32);
    }

    // Each u32 is 4 bytes
    assert_eq!(serialized.len(), 8);

    let (v1, _) = u32::from_bytes(&serialized[0..4]).unwrap();
    let (v2, _) = u32::from_bytes(&serialized[4..8]).unwrap();
    assert_eq!(v1, 1);
    assert_eq!(v2, 2);
}

// --- decode_frame malformed input tests ---

#[test]
fn decode_frame_empty_input() {
    let result = decode_frame::<u32>(&[]);
    assert!(result.is_err());
}

#[test]
fn decode_frame_truncated_header() {
    // Only 4 bytes — not enough for producer_id (8) + payload_len (4)
    let result = decode_frame::<u32>(&[0; 4]);
    assert!(result.is_err());
}

#[test]
fn decode_frame_payload_length_mismatch() {
    // Valid header: producer_id=0 (8 bytes LE), payload_len=100 (4 bytes LE), but no payload
    let mut buf = [0u8; 12];
    // payload_len = 100 at offset 8
    buf[8] = 100;
    let result = decode_frame::<u32>(&buf);
    assert!(result.is_err());
}

#[test]
fn decode_frame_trailing_bytes_rejected() {
    // Build a valid frame, then append garbage
    let mut s = FramedSpout::new(1, CollectSpout::<Vec<u8>>::new());
    s.send(42u32).unwrap();
    let mut frame = s.into_inner().into_items().remove(0);
    frame.push(0xFF); // trailing garbage
    let result = decode_frame::<u32>(&frame);
    assert!(result.is_err());
}

#[test]
fn decode_frame_valid_roundtrip() {
    let mut s = FramedSpout::new(99, CollectSpout::<Vec<u8>>::new());
    s.send(12345u32).unwrap();
    let frame = &s.inner().items()[0];
    let (id, val) = decode_frame::<u32>(frame).unwrap();
    assert_eq!(id, 99);
    assert_eq!(val, 12345);
}

// --- decode_batch malformed input tests ---

#[test]
fn decode_batch_empty_input() {
    let result = decode_batch::<u32>(&[]);
    assert!(result.is_err());
}

#[test]
fn decode_batch_zero_threshold_rejected() {
    // threshold = 0 (8 bytes LE zeros) + empty vec (varint 0)
    let mut buf = [0u8; 9];
    buf[8] = 0; // varint 0 for empty vec
    let result = decode_batch::<u32>(&buf);
    assert!(result.is_err());
}

#[test]
fn decode_batch_trailing_bytes_rejected() {
    let mut s: BatchSpout<u32, CollectSpout<Vec<u32>>> = BatchSpout::new(5, CollectSpout::new());
    s.send(1).unwrap();
    let mut bytes = s.to_vec().unwrap();
    bytes.push(0xFF); // trailing garbage
    let result = decode_batch::<u32>(&bytes);
    assert!(result.is_err());
}

#[test]
fn decode_batch_valid_roundtrip() {
    let mut s: BatchSpout<u32, CollectSpout<Vec<u32>>> = BatchSpout::new(10, CollectSpout::new());
    s.send(1).unwrap();
    s.send(2).unwrap();
    s.send(3).unwrap();
    let bytes = s.to_vec().unwrap();
    let (threshold, buffer) = decode_batch::<u32>(&bytes).unwrap();
    assert_eq!(threshold, 10);
    assert_eq!(buffer, vec![1, 2, 3]);
}

#[test]
fn decode_batch_oversized_vec_length() {
    // threshold = 1 (valid), then a varint claiming u32::MAX elements
    let mut buf = vec![0u8; 8]; // threshold = 0 in LE... wait, need threshold >= 1
    buf[0] = 1; // threshold = 1 in LE (usize serialized as u64)
    buf.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0x0F]); // varint u32::MAX
    let result = decode_batch::<u32>(&buf);
    assert!(result.is_err());
}
