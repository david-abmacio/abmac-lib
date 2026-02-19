extern crate std;

use std::{vec, vec::Vec};

use crate::SpillRing;
use spout::{BatchSpout, CollectSpout, FnSpout};

#[test]
fn fn_spout_receives_evicted() {
    let evicted = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let evicted_clone = evicted.clone();

    let ring = SpillRing::<i32, 2, _>::builder()
        .spout(FnSpout::new(move |x: i32| {
            evicted_clone.lock().unwrap().push(x);
        }))
        .build();

    ring.push(1);
    ring.push(2);
    ring.push(3); // Evicts 1 to spout

    // Spout should have received 1 immediately
    assert_eq!(*evicted.lock().unwrap(), vec![1]);
}

#[test]
fn batch_spout_with_ring_chain() {
    // ring -> BatchSpout -> CollectSpout
    // Reduces cascade traffic
    let batch_spout: BatchSpout<i32, CollectSpout<Vec<i32>>> =
        BatchSpout::new(100, CollectSpout::new());
    let ring = SpillRing::<i32, 4, _>::builder().spout(batch_spout).build();

    for i in 0..1000 {
        ring.push(i);
    }

    // 996 evictions, batch size 100 → 9 full batches flushed, 96 buffered
    let batches = ring.spout().inner().items();
    assert_eq!(batches.len(), 9);
    assert!(batches.iter().all(|b| b.len() == 100));

    // Total evicted items should be 0..996 (first 996 of 1000)
    let total_evicted: usize = batches.iter().map(Vec::len).sum();
    assert_eq!(total_evicted, 900); // 9 * 100 flushed so far

    // Ring should still have last 4 items
    assert_eq!(ring.len(), 4);
    assert_eq!(ring.pop(), Some(996));
    assert_eq!(ring.pop(), Some(997));
    assert_eq!(ring.pop(), Some(998));
    assert_eq!(ring.pop(), Some(999));
}

#[cfg(feature = "std")]
mod channel_spout_tests {
    use spout::{ChannelSpout, Spout};
    use std::sync::mpsc;

    #[test]
    fn channel_spout_accessors() {
        let (tx, rx) = mpsc::channel();
        let spout = ChannelSpout::new(tx);

        // Test sender() accessor
        spout.sender().send(42).unwrap();
        assert_eq!(rx.recv(), Ok(42));

        // Test into_sender()
        let sender = spout.into_sender();
        sender.send(99).unwrap();
        assert_eq!(rx.recv(), Ok(99));
    }

    #[test]
    fn channel_spout_ignores_disconnected_receiver() {
        let (tx, rx) = mpsc::channel::<i32>();
        let mut spout = ChannelSpout::new(tx);

        // Drop receiver
        drop(rx);

        // send should not panic
        let _ = spout.send(1);
        let _ = spout.send(2);
    }
}
