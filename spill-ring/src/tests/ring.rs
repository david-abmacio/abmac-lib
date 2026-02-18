extern crate std;

use core::mem::MaybeUninit;
use std::{vec, vec::Vec};

use crate::SpillRing;
use spout::CollectSpout;

#[test]
fn new_ring_is_empty() {
    let ring: SpillRing<i32, 4> = SpillRing::new();
    assert!(ring.is_empty());
    assert!(!ring.is_full());
    assert_eq!(ring.len(), 0);
    assert_eq!(ring.capacity(), 4);

    // Default delegates to new
    let ring2: SpillRing<i32, 4> = SpillRing::default();
    assert!(ring2.is_empty());
    assert_eq!(ring2.capacity(), 4);
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
    let sink = CollectSpout::new();
    let mut ring = SpillRing::<i32, 4, _>::with_sink(sink);

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

    for item in &mut ring {
        *item *= 10;
    }

    let items: Vec<i32> = ring.iter().copied().collect();
    assert_eq!(items, vec![10, 20, 30]);
}

#[test]
fn flush_clears_buffer() {
    let sink = CollectSpout::new();
    let mut ring = SpillRing::<i32, 4, _>::with_sink(sink);

    ring.push(1);
    ring.push(2);
    ring.push(3);

    let _ = ring.flush();

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
    impl spout::Spout<i32> for CountingSink {
        type Error = core::convert::Infallible;

        fn send(&mut self, _item: i32) -> Result<(), Self::Error> {
            SINK_COUNT.fetch_add(1, Ordering::SeqCst);
            Ok(())
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
fn push_and_flush() {
    let sink = CollectSpout::new();
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

// push_slice tests

#[test]
fn push_slice_empty() {
    let mut ring: SpillRing<u32, 8> = SpillRing::new();
    ring.push_slice(&[]);
    assert!(ring.is_empty());
}

#[test]
fn push_slice_fits_no_eviction() {
    let mut ring: SpillRing<u32, 8> = SpillRing::new();
    ring.push_slice(&[1, 2, 3, 4, 5]);
    assert_eq!(ring.len(), 5);
    for i in 1..=5 {
        assert_eq!(ring.pop_mut(), Some(i));
    }
}

#[test]
fn push_slice_exact_capacity() {
    let mut ring: SpillRing<u32, 4> = SpillRing::new();
    ring.push_slice(&[10, 20, 30, 40]);
    assert_eq!(ring.len(), 4);
    assert!(ring.is_full());
    assert_eq!(ring.pop_mut(), Some(10));
    assert_eq!(ring.pop_mut(), Some(20));
    assert_eq!(ring.pop_mut(), Some(30));
    assert_eq!(ring.pop_mut(), Some(40));
}

#[test]
fn push_slice_partial_eviction() {
    let sink = CollectSpout::new();
    let mut ring = SpillRing::<u32, 4, _>::with_sink(sink);
    ring.push_slice(&[1, 2, 3]); // 3 items, 1 free
    ring.push_slice(&[4, 5, 6]); // needs 3, only 1 free → evict 2
    assert_eq!(ring.sink().items(), vec![1, 2]);
    assert_eq!(ring.len(), 4);
    assert_eq!(ring.pop_mut(), Some(3));
    assert_eq!(ring.pop_mut(), Some(4));
    assert_eq!(ring.pop_mut(), Some(5));
    assert_eq!(ring.pop_mut(), Some(6));
}

#[test]
fn push_slice_exceeds_capacity() {
    let sink = CollectSpout::new();
    let mut ring = SpillRing::<u32, 4, _>::with_sink(sink);
    ring.push_mut(100); // pre-fill with 1 item
    ring.push_slice(&[1, 2, 3, 4, 5, 6, 7, 8]); // 8 items into cap-4 ring
    // Ring had [100], slice is 8 items (> N=4).
    // Phase 1: evict ring contents [100], send excess [1,2,3,4] to spout.
    // Phase 2: keep=[5,6,7,8] fills ring exactly.
    assert_eq!(ring.sink().items(), vec![100, 1, 2, 3, 4]);
    assert_eq!(ring.len(), 4);
    assert_eq!(ring.pop_mut(), Some(5));
    assert_eq!(ring.pop_mut(), Some(6));
    assert_eq!(ring.pop_mut(), Some(7));
    assert_eq!(ring.pop_mut(), Some(8));
}

#[test]
fn push_slice_wraparound() {
    let mut ring: SpillRing<u32, 8> = SpillRing::new();
    // Fill 6 items, pop 6 → head=6, tail=6 (near end of buffer)
    ring.push_slice(&[0; 6]);
    for _ in 0..6 {
        let _ = ring.pop_mut();
    }
    // Now push 5 items starting at tail_idx=6: wraps around buffer end
    ring.push_slice(&[10, 20, 30, 40, 50]);
    assert_eq!(ring.len(), 5);
    assert_eq!(ring.pop_mut(), Some(10));
    assert_eq!(ring.pop_mut(), Some(20));
    assert_eq!(ring.pop_mut(), Some(30));
    assert_eq!(ring.pop_mut(), Some(40));
    assert_eq!(ring.pop_mut(), Some(50));
}

#[test]
fn push_slice_wraparound_with_eviction() {
    let sink = CollectSpout::new();
    let mut ring = SpillRing::<u32, 4, _>::with_sink(sink);
    // Fill 3 items, pop 2 → head=2, tail=3, len=1 at slot[2]
    ring.push_slice(&[0, 0, 99]);
    let _ = ring.pop_mut(); // 0
    let _ = ring.pop_mut(); // 0
    // tail_idx=3, 1 item in ring (99 at slot[2]), 3 free
    // Push 4 items: needs 4, free=3, evict 1 (the 99)
    ring.push_slice(&[10, 20, 30, 40]);
    assert_eq!(ring.sink().items(), vec![99]);
    assert_eq!(ring.len(), 4);
    // Items wrap: slot[3]=10, slot[0]=20, slot[1]=30, slot[2]=40
    assert_eq!(ring.pop_mut(), Some(10));
    assert_eq!(ring.pop_mut(), Some(20));
    assert_eq!(ring.pop_mut(), Some(30));
    assert_eq!(ring.pop_mut(), Some(40));
}

// pop_slice tests

#[test]
fn pop_slice_empty() {
    let mut ring: SpillRing<u32, 8> = SpillRing::new();
    let mut buf = [MaybeUninit::uninit(); 4];
    assert_eq!(ring.pop_slice(&mut buf), 0);
}

#[test]
fn pop_slice_partial() {
    let mut ring: SpillRing<u32, 8> = SpillRing::new();
    ring.push_slice(&[10, 20, 30]);
    let mut buf = [MaybeUninit::uninit(); 2];
    let n = ring.pop_slice(&mut buf);
    assert_eq!(n, 2);
    assert_eq!(unsafe { buf[0].assume_init() }, 10);
    assert_eq!(unsafe { buf[1].assume_init() }, 20);
    assert_eq!(ring.len(), 1);
    assert_eq!(ring.pop_mut(), Some(30));
}

#[test]
fn pop_slice_exact() {
    let mut ring: SpillRing<u32, 8> = SpillRing::new();
    ring.push_slice(&[1, 2, 3, 4]);
    let mut buf = [MaybeUninit::uninit(); 4];
    let n = ring.pop_slice(&mut buf);
    assert_eq!(n, 4);
    let vals: Vec<u32> = buf.iter().map(|m| unsafe { m.assume_init() }).collect();
    assert_eq!(vals, vec![1, 2, 3, 4]);
    assert!(ring.is_empty());
}

#[test]
fn pop_slice_more_than_available() {
    let mut ring: SpillRing<u32, 8> = SpillRing::new();
    ring.push_slice(&[5, 6]);
    let mut buf = [MaybeUninit::uninit(); 10];
    let n = ring.pop_slice(&mut buf);
    assert_eq!(n, 2);
    assert_eq!(unsafe { buf[0].assume_init() }, 5);
    assert_eq!(unsafe { buf[1].assume_init() }, 6);
    assert!(ring.is_empty());
}

#[test]
fn pop_slice_wraparound() {
    let mut ring: SpillRing<u32, 4> = SpillRing::new();
    // Fill and pop to advance head past slot 0.
    ring.push_slice(&[0, 0, 0]);
    let _ = ring.pop_mut(); // head=1
    let _ = ring.pop_mut(); // head=2
    let _ = ring.pop_mut(); // head=3, tail=3
    // Now push 4 items wrapping around: slots [3,0,1,2]
    ring.push_slice(&[10, 20, 30, 40]);
    let mut buf = [MaybeUninit::uninit(); 4];
    let n = ring.pop_slice(&mut buf);
    assert_eq!(n, 4);
    let vals: Vec<u32> = buf.iter().map(|m| unsafe { m.assume_init() }).collect();
    assert_eq!(vals, vec![10, 20, 30, 40]);
}

#[test]
fn pop_slice_empty_buf() {
    let mut ring: SpillRing<u32, 8> = SpillRing::new();
    ring.push_slice(&[1, 2, 3]);
    let mut buf: [MaybeUninit<u32>; 0] = [];
    assert_eq!(ring.pop_slice(&mut buf), 0);
    assert_eq!(ring.len(), 3);
}

// Builder tests

#[test]
fn spill_ring_builder_default() {
    let ring = SpillRing::<u64, 16>::builder().build();
    assert!(ring.is_empty());
    assert_eq!(ring.capacity(), 16);
    ring.push(42);
    assert_eq!(ring.pop(), Some(42));
}

#[test]
fn spill_ring_builder_cold() {
    let ring = SpillRing::<u64, 16>::builder().cold().build();
    assert!(ring.is_empty());
    ring.push(1);
    assert_eq!(ring.pop(), Some(1));
}

#[test]
fn spill_ring_builder_with_sink() {
    let sink = CollectSpout::new();
    let mut ring = SpillRing::<u64, 4>::builder().sink(sink).build();
    // Overflow to trigger spout
    for i in 0..8 {
        ring.push(i);
    }
    assert_eq!(ring.len(), 4);
    // First 4 items evicted to sink, last 4 remain in ring
    assert_eq!(ring.sink().items(), vec![0, 1, 2, 3]);
    assert_eq!(ring.pop(), Some(4));
    assert_eq!(ring.pop(), Some(5));
    assert_eq!(ring.pop(), Some(6));
    assert_eq!(ring.pop(), Some(7));
}

#[test]
fn spill_ring_builder_cold_with_sink() {
    let sink = CollectSpout::new();
    let ring = SpillRing::<u64, 4>::builder().sink(sink).cold().build();
    ring.push(1);
    assert_eq!(ring.pop(), Some(1));
}

// --- Missing coverage tests ---

#[test]
fn push_mut_with_eviction() {
    let sink = CollectSpout::new();
    let mut ring = SpillRing::<i32, 4, _>::with_sink(sink);

    for i in 0..6 {
        ring.push_mut(i);
    }

    // First 2 evicted to sink, last 4 in ring
    assert_eq!(ring.sink().items(), vec![0, 1]);
    assert_eq!(ring.pop_mut(), Some(2));
    assert_eq!(ring.pop_mut(), Some(3));
    assert_eq!(ring.pop_mut(), Some(4));
    assert_eq!(ring.pop_mut(), Some(5));
}

#[test]
fn pop_mut_empty_and_wraparound() {
    let mut ring: SpillRing<i32, 4> = SpillRing::new();

    // Empty ring
    assert_eq!(ring.pop_mut(), None);

    // Fill, pop all, refill to trigger wraparound
    for i in 0..3 {
        ring.push_mut(i);
    }
    assert_eq!(ring.pop_mut(), Some(0));
    assert_eq!(ring.pop_mut(), Some(1));
    assert_eq!(ring.pop_mut(), Some(2));
    assert_eq!(ring.pop_mut(), None);

    // Wraparound: head=3, push 4 items wrapping around
    for i in 10..14 {
        ring.push_mut(i);
    }
    assert_eq!(ring.pop_mut(), Some(10));
    assert_eq!(ring.pop_mut(), Some(11));
    assert_eq!(ring.pop_mut(), Some(12));
    assert_eq!(ring.pop_mut(), Some(13));
}

#[test]
fn peek_back_after_wraparound() {
    let ring: SpillRing<i32, 4> = SpillRing::new();

    // Fill and overflow to wrap around
    for i in 0..6 {
        ring.push(i);
    }
    // Ring contains [2, 3, 4, 5], tail has wrapped
    assert_eq!(ring.peek(), Some(&2));
    assert_eq!(ring.peek_back(), Some(&5));
}

#[test]
fn get_after_wraparound() {
    let ring: SpillRing<i32, 4> = SpillRing::new();

    // Fill and overflow: ring contains [4, 5, 6, 7]
    for i in 0..8 {
        ring.push(i);
    }
    assert_eq!(ring.get(0), Some(&4));
    assert_eq!(ring.get(1), Some(&5));
    assert_eq!(ring.get(2), Some(&6));
    assert_eq!(ring.get(3), Some(&7));
    assert_eq!(ring.get(4), None);
}

#[test]
fn with_sink_cold_eviction() {
    let sink = CollectSpout::new();
    let mut ring = SpillRing::<i32, 4, _>::with_sink_cold(sink);

    for i in 0..8 {
        ring.push(i);
    }
    // Cold-constructed ring should handle eviction correctly
    assert_eq!(ring.sink().items(), vec![0, 1, 2, 3]);
    assert_eq!(ring.len(), 4);
    assert_eq!(ring.pop(), Some(4));
    assert_eq!(ring.pop(), Some(5));
    assert_eq!(ring.pop(), Some(6));
    assert_eq!(ring.pop(), Some(7));
}

#[test]
fn cold_constructor_wraparound() {
    let mut ring = SpillRing::<i32, 4>::builder().cold().build();

    // Fill, pop, refill to test wraparound on cold ring
    for i in 0..3 {
        ring.push_mut(i);
    }
    let _ = ring.pop_mut();
    let _ = ring.pop_mut();
    let _ = ring.pop_mut();
    // head=3, tail=3 — near buffer end
    for i in 10..14 {
        ring.push_mut(i);
    }
    assert_eq!(ring.pop_mut(), Some(10));
    assert_eq!(ring.pop_mut(), Some(11));
    assert_eq!(ring.pop_mut(), Some(12));
    assert_eq!(ring.pop_mut(), Some(13));
}

#[test]
fn drain_size_hint() {
    let mut ring: SpillRing<i32, 8> = SpillRing::new();
    ring.push(1);
    ring.push(2);
    ring.push(3);

    let drain = ring.drain();
    assert_eq!(drain.len(), 3);
    assert_eq!(drain.size_hint(), (3, Some(3)));
}

#[test]
fn fused_iterator_behavior() {
    let ring: SpillRing<i32, 4> = SpillRing::new();
    ring.push(1);

    let mut iter = ring.iter();
    assert_eq!(iter.next(), Some(&1));
    assert_eq!(iter.next(), None);
    // FusedIterator: next() after exhaustion still returns None
    assert_eq!(iter.next(), None);
    assert_eq!(iter.next(), None);
}

#[test]
fn iter_mut_after_wraparound() {
    let mut ring: SpillRing<i32, 4> = SpillRing::new();

    // Fill and overflow to wrap around
    for i in 0..6 {
        ring.push(i);
    }
    // Ring contains [2, 3, 4, 5] with wrapped indices

    for item in &mut ring {
        *item *= 10;
    }

    let items: Vec<i32> = ring.iter().copied().collect();
    assert_eq!(items, vec![20, 30, 40, 50]);
}

#[test]
fn into_iter_ref() {
    let ring: SpillRing<i32, 4> = SpillRing::new();
    ring.push(1);
    ring.push(2);
    ring.push(3);

    let items: Vec<&i32> = (&ring).into_iter().collect();
    assert_eq!(items, vec![&1, &2, &3]);
}

#[test]
fn into_iter_mut_ref() {
    let mut ring: SpillRing<i32, 4> = SpillRing::new();
    ring.push(1);
    ring.push(2);
    ring.push(3);

    for item in &mut ring {
        *item += 100;
    }

    let items: Vec<i32> = ring.iter().copied().collect();
    assert_eq!(items, vec![101, 102, 103]);
}

#[test]
fn extend_overflow_to_sink() {
    let sink = CollectSpout::new();
    let mut ring = SpillRing::<i32, 4, _>::with_sink(sink);
    ring.extend(0..10);

    // First 6 evicted to sink, last 4 in ring
    assert_eq!(ring.sink().items(), vec![0, 1, 2, 3, 4, 5]);
    assert_eq!(ring.len(), 4);
    assert_eq!(ring.pop(), Some(6));
}

#[test]
fn spill_ring_as_spout() {
    use spout::Spout;

    let mut ring = SpillRing::<i32, 4>::new();

    // Use the Spout trait's send() method
    Spout::send(&mut ring, 10).unwrap();
    Spout::send(&mut ring, 20).unwrap();
    Spout::send(&mut ring, 30).unwrap();

    assert_eq!(ring.len(), 3);
    assert_eq!(ring.pop(), Some(10));
    assert_eq!(ring.pop(), Some(20));
    assert_eq!(ring.pop(), Some(30));

    // Spout::flush() should flush ring contents
    ring.push(1);
    ring.push(2);
    Spout::flush(&mut ring).unwrap();
    assert!(ring.is_empty());
}
