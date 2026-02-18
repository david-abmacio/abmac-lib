extern crate std;

use std::vec;
use std::vec::Vec;

use bytecast::{FromBytes, ToBytesExt};

use crate::{BatchSpout, CollectSpout, FramedSpout, Spout, decode_frame};

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
