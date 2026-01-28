extern crate std;

use std::{vec, vec::Vec};

use crate::SpillRing;
use spout::{BatchSink, CollectSink, FnSink};

#[test]
fn fn_sink_receives_evicted() {
    let evicted = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let evicted_clone = evicted.clone();

    let ring = SpillRing::<i32, 2, _>::with_sink(FnSink(move |x| {
        evicted_clone.lock().unwrap().push(x);
    }));

    ring.push(1);
    ring.push(2);
    ring.push(3); // Evicts 1 directly to sink

    // Sink should have received 1 immediately
    assert_eq!(*evicted.lock().unwrap(), vec![1]);
}

#[test]
fn batch_sink_with_ring_chain() {
    // ring -> BatchSink -> CollectSink
    // Reduces cascade traffic
    let batch_sink: BatchSink<i32, CollectSink<Vec<i32>>> = BatchSink::new(100, CollectSink::new());
    let ring = SpillRing::<i32, 4, _>::with_sink(batch_sink);

    for i in 0..1000 {
        ring.push(i);
    }

    // Evictions batched into groups of 100
    let batches = ring.sink().inner().items();
    assert!(batches.len() >= 9); // ~996 evictions / 100 = 9+ batches
    assert!(batches.iter().all(|b| b.len() <= 100));
}

#[cfg(feature = "std")]
mod channel_sink_tests {
    use crate::{ChannelSink, Sink, SpillRing};
    use std::sync::mpsc;

    #[test]
    fn channel_sink_sends_evicted_items() {
        let (tx, rx) = mpsc::channel();
        let ring = SpillRing::<i32, 4, _>::with_sink(ChannelSink::new(tx));

        // Fill ring
        ring.push(1);
        ring.push(2);
        ring.push(3);
        ring.push(4);

        // No evictions yet
        assert!(rx.try_recv().is_err());

        // Trigger evictions
        ring.push(5); // evicts 1
        ring.push(6); // evicts 2

        assert_eq!(rx.try_recv(), Ok(1));
        assert_eq!(rx.try_recv(), Ok(2));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn channel_sink_with_flush() {
        let (tx, rx) = mpsc::channel();
        let mut ring = SpillRing::<i32, 4, _>::with_sink(ChannelSink::new(tx));

        ring.push(10);
        ring.push(20);
        ring.push(30);

        ring.flush();

        let items: Vec<_> = rx.try_iter().collect();
        assert_eq!(items, vec![10, 20, 30]);
    }

    #[test]
    fn channel_sink_drop_sends_remaining() {
        let (tx, rx) = mpsc::channel();

        {
            let ring = SpillRing::<i32, 4, _>::with_sink(ChannelSink::new(tx));
            ring.push(1);
            ring.push(2);
            // Ring dropped here, flushes to sink
        }

        let items: Vec<_> = rx.try_iter().collect();
        assert_eq!(items, vec![1, 2]);
    }

    #[test]
    fn channel_sink_accessors() {
        let (tx, rx) = mpsc::channel();
        let sink = ChannelSink::new(tx);

        // Test sender() accessor
        sink.sender().send(42).unwrap();
        assert_eq!(rx.recv(), Ok(42));

        // Test into_sender()
        let sender = sink.into_sender();
        sender.send(99).unwrap();
        assert_eq!(rx.recv(), Ok(99));
    }

    #[test]
    fn channel_sink_ignores_disconnected_receiver() {
        let (tx, rx) = mpsc::channel::<i32>();
        let mut sink = ChannelSink::new(tx);

        // Drop receiver
        drop(rx);

        // send should not panic
        sink.send(1);
        sink.send(2);
    }

    #[test]
    fn channel_sink_mpsc_pattern() {
        // Multiple rings sending to one receiver
        let (tx, rx) = mpsc::channel();

        let ring1 = SpillRing::<i32, 2, _>::with_sink(ChannelSink::new(tx.clone()));
        let ring2 = SpillRing::<i32, 2, _>::with_sink(ChannelSink::new(tx.clone()));
        drop(tx); // Drop original sender

        // Fill and overflow both rings
        ring1.push(10);
        ring1.push(11);
        ring1.push(12); // evicts 10

        ring2.push(20);
        ring2.push(21);
        ring2.push(22); // evicts 20

        // Both evictions should arrive at receiver
        let mut evicted: Vec<_> = rx.try_iter().collect();
        evicted.sort();
        assert_eq!(evicted, vec![10, 20]);
    }
}
