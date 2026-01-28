extern crate std;

use std::{vec, vec::Vec};

use crate::SpillRing;
use spout::{CollectSink, Sink};

#[test]
fn new_ring_is_empty() {
    let ring: SpillRing<i32, 4> = SpillRing::new();
    assert!(ring.is_empty());
    assert!(!ring.is_full());
    assert_eq!(ring.len(), 0);
    assert_eq!(ring.capacity(), 4);
}

#[test]
fn push_and_pop() {
    let ring: SpillRing<i32, 4> = SpillRing::new();

    ring.push(1);
    ring.push(2);
    ring.push(3);

    assert_eq!(ring.len(), 3);
    assert_eq!(ring.pop(), Some(1));
    assert_eq!(ring.pop(), Some(2));
    assert_eq!(ring.pop(), Some(3));
    assert_eq!(ring.pop(), None);
}

#[test]
fn eviction_to_sink() {
    // N=4 main buffer, items evicted directly to sink
    let sink = CollectSink::new();
    let ring = SpillRing::<i32, 4, _>::with_sink(sink);

    ring.push(1);
    ring.push(2);
    ring.push(3);
    ring.push(4);
    assert!(ring.sink().items().is_empty()); // Nothing evicted yet

    ring.push(5); // Evicts 1 directly to sink
    assert_eq!(ring.sink().items(), vec![1]);

    ring.push(6); // Evicts 2 directly to sink
    assert_eq!(ring.sink().items(), vec![1, 2]);

    // Ring now contains [3, 4, 5, 6]
    assert_eq!(ring.pop(), Some(3));
    assert_eq!(ring.pop(), Some(4));
    assert_eq!(ring.pop(), Some(5));
    assert_eq!(ring.pop(), Some(6));
}

#[test]
fn flush_to_sink() {
    let sink = CollectSink::new();
    let mut ring = SpillRing::<i32, 4, _>::with_sink(sink);

    ring.push(1);
    ring.push(2);
    ring.push(3);
    ring.push(4);
    ring.push(5); // Evicts 1 directly to sink
    ring.push(6); // Evicts 2 directly to sink

    // Sink already has [1, 2] from direct eviction
    assert_eq!(ring.sink().items(), vec![1, 2]);

    // Flush remaining buffer items to sink
    let count = ring.flush();
    assert_eq!(count, 4); // 3, 4, 5, 6
    assert!(ring.is_empty());
    assert_eq!(ring.sink().items(), vec![1, 2, 3, 4, 5, 6]);
}

#[test]
fn peek_oldest_and_newest() {
    let ring: SpillRing<i32, 4> = SpillRing::new();

    assert_eq!(ring.peek(), None);
    assert_eq!(ring.peek_back(), None);

    ring.push(1);
    assert_eq!(ring.peek(), Some(&1));
    assert_eq!(ring.peek_back(), Some(&1));

    ring.push(2);
    ring.push(3);
    assert_eq!(ring.peek(), Some(&1));
    assert_eq!(ring.peek_back(), Some(&3));
}

#[test]
fn iteration() {
    let ring: SpillRing<i32, 4> = SpillRing::new();

    ring.push(1);
    ring.push(2);
    ring.push(3);

    let items: Vec<i32> = ring.iter().copied().collect();
    assert_eq!(items, std::vec![1, 2, 3]);
}

#[test]
fn iter_mut() {
    let mut ring: SpillRing<i32, 4> = SpillRing::new();

    ring.push(1);
    ring.push(2);
    ring.push(3);

    for item in ring.iter_mut() {
        *item *= 10;
    }

    let items: Vec<i32> = ring.iter().copied().collect();
    assert_eq!(items, vec![10, 20, 30]);
}

#[test]
fn flush_clears_buffer() {
    let sink = CollectSink::new();
    let mut ring = SpillRing::<i32, 4, _>::with_sink(sink);

    ring.push(1);
    ring.push(2);
    ring.push(3);

    ring.flush();

    assert!(ring.is_empty());
    assert_eq!(ring.sink().items(), vec![1, 2, 3]);
}

#[test]
fn wraparound() {
    let ring: SpillRing<i32, 4> = SpillRing::new();

    // Fill and wrap around multiple times
    for i in 0..12 {
        ring.push(i);
    }

    // Should contain [8, 9, 10, 11]
    assert_eq!(ring.pop(), Some(8));
    assert_eq!(ring.pop(), Some(9));
    assert_eq!(ring.pop(), Some(10));
    assert_eq!(ring.pop(), Some(11));
}

#[test]
fn get_by_index() {
    let ring: SpillRing<i32, 4> = SpillRing::new();

    ring.push(10);
    ring.push(20);
    ring.push(30);

    assert_eq!(ring.get(0), Some(&10));
    assert_eq!(ring.get(1), Some(&20));
    assert_eq!(ring.get(2), Some(&30));
    assert_eq!(ring.get(3), None);
}

#[test]
fn drop_flushes_to_sink() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SINK_COUNT: AtomicUsize = AtomicUsize::new(0);

    struct CountingSink;
    impl spout::Sink<i32> for CountingSink {
        fn send(&mut self, _item: i32) {
            SINK_COUNT.fetch_add(1, Ordering::SeqCst);
        }
    }

    SINK_COUNT.store(0, Ordering::SeqCst);

    {
        let ring = SpillRing::<i32, 4, _>::with_sink(CountingSink);
        ring.push(1);
        ring.push(2);
        ring.push(3);
        // 3 items in ring, none sent to sink yet
        assert_eq!(SINK_COUNT.load(Ordering::SeqCst), 0);
    }
    // Ring dropped, all 3 items should be flushed to sink
    assert_eq!(SINK_COUNT.load(Ordering::SeqCst), 3);
}

#[test]
fn drop_with_default_sink_drops_items() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

    struct DropCounter;
    impl Drop for DropCounter {
        fn drop(&mut self) {
            DROP_COUNT.fetch_add(1, Ordering::SeqCst);
        }
    }

    DROP_COUNT.store(0, Ordering::SeqCst);

    {
        let ring: SpillRing<DropCounter, 4> = SpillRing::new();
        ring.push(DropCounter);
        ring.push(DropCounter);
        ring.push(DropCounter);
        // 3 items in ring, none dropped yet
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 0);
    }
    // Ring dropped with DropSink, all 3 items should be dropped
    assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 3);
}

#[test]
fn overflow_sends_to_sink_immediately() {
    // N=2 main buffer, evicted items go directly to sink
    let sink = CollectSink::new();
    let ring = SpillRing::<i32, 2, _>::with_sink(sink);

    ring.push(1);
    ring.push(2);
    // Main buffer full: [1, 2]
    assert!(ring.sink().items().is_empty());

    ring.push(3); // Evicts 1 directly to sink
    assert_eq!(ring.sink().items(), vec![1]);

    ring.push(4); // Evicts 2 directly to sink
    assert_eq!(ring.sink().items(), vec![1, 2]);

    ring.push(5); // Evicts 3 directly to sink
    assert_eq!(ring.sink().items(), vec![1, 2, 3]);

    ring.push(6); // Evicts 4 directly to sink
    assert_eq!(ring.sink().items(), vec![1, 2, 3, 4]);

    // Main buffer: [5, 6]
    assert_eq!(ring.pop(), Some(5));
    assert_eq!(ring.pop(), Some(6));
}

#[test]
fn clear_drop_ignores_sink() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SINK_COUNT: AtomicUsize = AtomicUsize::new(0);

    struct CountingSink;
    impl spout::Sink<i32> for CountingSink {
        fn send(&mut self, _item: i32) {
            SINK_COUNT.fetch_add(1, Ordering::SeqCst);
        }
    }

    SINK_COUNT.store(0, Ordering::SeqCst);

    let ring = SpillRing::<i32, 4, _>::with_sink(CountingSink);
    ring.push(1);
    ring.push(2);
    ring.push(3);

    ring.clear_drop();

    assert!(ring.is_empty());
    // Sink should NOT have been called
    assert_eq!(SINK_COUNT.load(Ordering::SeqCst), 0);

    // Prevent drop from calling sink by clearing again
    ring.clear_drop();
}

#[test]
fn push_and_flush() {
    let sink = CollectSink::new();
    let mut ring = SpillRing::<i32, 4, _>::with_sink(sink);

    ring.push_and_flush(1);
    assert!(ring.is_empty());
    assert_eq!(ring.sink().items(), vec![1]);

    ring.push(2);
    ring.push(3);
    ring.push_and_flush(4);
    assert!(ring.is_empty());
    assert_eq!(ring.sink().items(), vec![1, 2, 3, 4]);
}

#[test]
fn clear_flushes_to_sink() {
    let sink = CollectSink::new();
    let mut ring = SpillRing::<i32, 4, _>::with_sink(sink);

    ring.push(1);
    ring.push(2);
    ring.push(3);

    ring.clear();

    assert!(ring.is_empty());
    assert_eq!(ring.sink().items(), vec![1, 2, 3]);
}

#[test]
fn default_creates_empty_ring() {
    let ring: SpillRing<i32, 4> = SpillRing::default();
    assert!(ring.is_empty());
    assert_eq!(ring.capacity(), 4);
}

#[test]
fn iter_nth() {
    let ring: SpillRing<i32, 8> = SpillRing::new();
    ring.push(10);
    ring.push(20);
    ring.push(30);
    ring.push(40);
    ring.push(50);

    let mut iter = ring.iter();

    // Skip 2, get 3rd element
    assert_eq!(iter.nth(2), Some(&30));
    // Next after nth should be 4th element
    assert_eq!(iter.next(), Some(&40));
    // nth beyond remaining
    assert_eq!(iter.nth(10), None);
}

#[test]
fn iter_mut_size_hint() {
    let mut ring: SpillRing<i32, 4> = SpillRing::new();
    ring.push(1);
    ring.push(2);
    ring.push(3);

    let iter = ring.iter_mut();
    assert_eq!(iter.size_hint(), (3, Some(3)));
}

#[test]
fn drain_removes_all_items() {
    let mut ring: SpillRing<i32, 4> = SpillRing::new();
    ring.push(1);
    ring.push(2);
    ring.push(3);

    let drained: Vec<_> = ring.drain().collect();
    assert_eq!(drained, vec![1, 2, 3]);
    assert!(ring.is_empty());
}

#[test]
fn extend_adds_items() {
    let mut ring: SpillRing<i32, 8> = SpillRing::new();
    ring.extend([1, 2, 3]);
    assert_eq!(ring.len(), 3);
    assert_eq!(ring.pop(), Some(1));
    assert_eq!(ring.pop(), Some(2));
    assert_eq!(ring.pop(), Some(3));
}

#[test]
fn extend_with_overflow_evicts() {
    let mut ring: SpillRing<i32, 4> = SpillRing::new();
    ring.extend(0..10);
    // Only last 4 items remain (6, 7, 8, 9)
    assert_eq!(ring.len(), 4);
    assert_eq!(ring.pop(), Some(6));
}
