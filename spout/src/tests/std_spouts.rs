extern crate std;

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::vec;

use crate::{ChannelSpout, CollectSpout, Spout, SyncChannelSpout};

// --- ChannelSpout tests ---

#[test]
fn channel_spout_sends_items() {
    let (tx, rx) = mpsc::channel();
    let mut s = ChannelSpout::new(tx);

    s.send(1).unwrap();
    s.send(2).unwrap();
    s.send(3).unwrap();

    assert_eq!(rx.recv().unwrap(), 1);
    assert_eq!(rx.recv().unwrap(), 2);
    assert_eq!(rx.recv().unwrap(), 3);
}

#[test]
fn channel_spout_returns_error_on_disconnected_receiver() {
    let (tx, rx) = mpsc::channel::<i32>();
    let mut s = ChannelSpout::new(tx);

    drop(rx);

    assert!(s.send(1).is_err());
    assert!(s.send(2).is_err());
}

#[test]
fn channel_spout_sender_accessor() {
    let (tx, _rx) = mpsc::channel::<i32>();
    let s = ChannelSpout::new(tx);

    // Verify we can access the sender
    let _sender = s.sender();
}

#[test]
fn channel_spout_into_sender() {
    let (tx, rx) = mpsc::channel::<i32>();
    let s = ChannelSpout::new(tx);

    let sender = s.into_sender();
    sender.send(42).unwrap();
    assert_eq!(rx.recv().unwrap(), 42);
}

// --- SyncChannelSpout tests ---

#[test]
fn sync_channel_spout_sends_items() {
    let (tx, rx) = mpsc::sync_channel(4);
    let mut s = SyncChannelSpout::new(tx);

    s.send(1).unwrap();
    s.send(2).unwrap();
    s.send(3).unwrap();

    assert_eq!(rx.recv().unwrap(), 1);
    assert_eq!(rx.recv().unwrap(), 2);
    assert_eq!(rx.recv().unwrap(), 3);
}

#[test]
fn sync_channel_spout_blocks_when_full() {
    let (tx, rx) = mpsc::sync_channel(2);
    let mut s = SyncChannelSpout::new(tx);

    s.send(1).unwrap();
    s.send(2).unwrap();
    // Channel is now full — send in a thread so we can unblock it
    let handle = thread::spawn(move || {
        s.send(3).unwrap(); // blocks until receiver drains
        s
    });

    // Drain one to unblock the sender
    assert_eq!(rx.recv().unwrap(), 1);

    let _s = handle.join().unwrap();
    assert_eq!(rx.recv().unwrap(), 2);
    assert_eq!(rx.recv().unwrap(), 3);
}

#[test]
fn sync_channel_spout_returns_error_on_disconnected_receiver() {
    let (tx, rx) = mpsc::sync_channel::<i32>(4);
    let mut s = SyncChannelSpout::new(tx);

    drop(rx);

    assert!(s.send(1).is_err());
}

#[test]
fn sync_channel_spout_into_sender() {
    let (tx, rx) = mpsc::sync_channel::<i32>(4);
    let s = SyncChannelSpout::new(tx);

    let sender = s.into_sender();
    sender.send(42).unwrap();
    assert_eq!(rx.recv().unwrap(), 42);
}

// --- Arc<Mutex<S>> tests ---

#[test]
fn arc_mutex_spout_sends_items() {
    let s = Arc::new(Mutex::new(CollectSpout::new()));
    let mut handle = s.clone();

    handle.send(1).unwrap();
    handle.send(2).unwrap();
    handle.send(3).unwrap();

    assert_eq!(s.lock().unwrap().items(), vec![1, 2, 3]);
}

#[test]
fn arc_mutex_spout_flush_delegates() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static FLUSH_COUNT: AtomicUsize = AtomicUsize::new(0);

    struct FlushTracker;
    impl Spout<i32> for FlushTracker {
        type Error = core::convert::Infallible;
        fn send(&mut self, _item: i32) -> Result<(), Self::Error> {
            Ok(())
        }
        fn flush(&mut self) -> Result<(), Self::Error> {
            FLUSH_COUNT.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    FLUSH_COUNT.store(0, Ordering::SeqCst);

    let mut s = Arc::new(Mutex::new(FlushTracker));
    s.flush().unwrap();
    assert_eq!(FLUSH_COUNT.load(Ordering::SeqCst), 1);
}

#[test]
fn arc_mutex_spout_shared_across_threads() {
    let s = Arc::new(Mutex::new(CollectSpout::new()));

    thread::scope(|scope| {
        for i in 0..4 {
            let mut handle = s.clone();
            scope.spawn(move || {
                for j in 0..10 {
                    handle.send(i * 100 + j).unwrap();
                }
            });
        }
    });

    assert_eq!(s.lock().unwrap().items().len(), 40);
}
