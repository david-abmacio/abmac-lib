extern crate std;

mod producer_spout;

#[cfg(feature = "std")]
mod std_spouts;

#[cfg(feature = "bytecast")]
mod bytecast_spouts;

use std::{vec, vec::Vec};

use crate::{BatchSpout, CollectSpout, DropSpout, FlushFn, FnSpout, ReduceSpout, Spout, spout};

#[test]
fn drop_spout_accepts_items() {
    let mut s = DropSpout;
    let _ = s.send(1);
    let _ = s.send(2);
    let _ = s.send(3);
    // Items are dropped, no way to verify except that it compiles
}

#[test]
fn fn_spout_calls_closure() {
    let mut collected = Vec::new();
    {
        let mut s = FnSpout::new(|x: i32| collected.push(x));
        let _ = s.send(1);
        let _ = s.send(2);
        let _ = s.send(3);
    }
    assert_eq!(collected, vec![1, 2, 3]);
}

#[test]
fn collect_spout_gathers_items() {
    let mut s = CollectSpout::new();
    let _ = s.send(10);
    let _ = s.send(20);
    let _ = s.send(30);
    assert_eq!(s.items(), vec![10, 20, 30]);
}

#[test]
fn spout_with_different_types() {
    let mut string_spout = CollectSpout::new();
    let _ = string_spout.send("hello");
    let _ = string_spout.send("world");
    assert_eq!(string_spout.items(), vec!["hello", "world"]);

    let mut tuple_spout = CollectSpout::new();
    let _ = tuple_spout.send((1, "a"));
    let _ = tuple_spout.send((2, "b"));
    assert_eq!(tuple_spout.items(), vec![(1, "a"), (2, "b")]);
}

#[test]
fn fn_flush_spout_calls_both_closures() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEND_COUNT: AtomicUsize = AtomicUsize::new(0);
    static FLUSH_COUNT: AtomicUsize = AtomicUsize::new(0);

    SEND_COUNT.store(0, Ordering::SeqCst);
    FLUSH_COUNT.store(0, Ordering::SeqCst);

    let mut s = spout(
        |_: i32| {
            SEND_COUNT.fetch_add(1, Ordering::SeqCst);
        },
        FlushFn(|| {
            FLUSH_COUNT.fetch_add(1, Ordering::SeqCst);
        }),
    );

    let _ = s.send(1);
    let _ = s.send(2);
    let _ = s.send(3);
    assert_eq!(SEND_COUNT.load(Ordering::SeqCst), 3);
    assert_eq!(FLUSH_COUNT.load(Ordering::SeqCst), 0);

    let _ = s.flush();
    assert_eq!(FLUSH_COUNT.load(Ordering::SeqCst), 1);

    let _ = s.flush();
    assert_eq!(FLUSH_COUNT.load(Ordering::SeqCst), 2);
}

#[test]
fn fn_flush_spout_with_unit_flush() {
    let mut collected = Vec::new();
    {
        // Using () for flush (no-op)
        let mut s = spout(|x: i32| collected.push(x), ());
        let _ = s.send(10);
        let _ = s.send(20);
        let _ = s.flush(); // Should be a no-op
    }
    assert_eq!(collected, vec![10, 20]);
}

#[test]
fn drop_spout_flush_is_noop() {
    let mut s = DropSpout;
    let _ = <DropSpout as Spout<i32>>::flush(&mut s); // Should not panic
}

#[test]
fn batch_spout_batches_items() {
    let mut s: BatchSpout<i32, CollectSpout<Vec<i32>>> = BatchSpout::new(3, CollectSpout::new());

    let _ = s.send(1);
    let _ = s.send(2);
    // Not yet forwarded
    assert_eq!(s.inner().items().len(), 0);
    assert_eq!(s.buffered(), 2);

    let _ = s.send(3);
    // Batch forwarded
    assert_eq!(s.inner().items(), vec![vec![1, 2, 3]]);
    assert_eq!(s.buffered(), 0);

    let _ = s.send(4);
    let _ = s.send(5);
    // Flush remaining
    let _ = s.flush();
    assert_eq!(s.into_inner().into_items(), vec![vec![1, 2, 3], vec![4, 5]]);
}

#[test]
fn batch_spout_exact_threshold() {
    let mut s: BatchSpout<i32, CollectSpout<Vec<i32>>> = BatchSpout::new(2, CollectSpout::new());

    let _ = s.send(1);
    let _ = s.send(2);
    let _ = s.send(3);
    let _ = s.send(4);

    assert_eq!(s.inner().items(), vec![vec![1, 2], vec![3, 4]]);
}

#[test]
fn batch_spout_flush_empty_is_noop() {
    let mut s: BatchSpout<i32, CollectSpout<Vec<i32>>> = BatchSpout::new(10, CollectSpout::new());
    let _ = s.flush();
    assert!(s.into_inner().into_items().is_empty());
}

#[test]
fn reduce_spout_reduces_batches() {
    let mut s = ReduceSpout::new(
        4,
        |batch: Vec<i32>| batch.iter().sum::<i32>(),
        CollectSpout::new(),
    );

    for i in 1..=8 {
        let _ = s.send(i);
    }
    let _ = s.flush();

    // [1+2+3+4=10, 5+6+7+8=26]
    assert_eq!(s.into_inner().into_items(), vec![10, 26]);
}

#[test]
fn reduce_spout_flush_partial() {
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    let mut s = ReduceSpout::new(5, |batch: Vec<i32>| batch.len() as i32, CollectSpout::new());

    let _ = s.send(1);
    let _ = s.send(2);
    let _ = s.send(3);
    let _ = s.flush();

    // Partial batch of 3 items
    assert_eq!(s.into_inner().into_items(), vec![3]);
}

#[test]
fn reduce_spout_type_transform() {
    use std::string::{String, ToString};
    // Transform i32 -> String
    let mut s = ReduceSpout::new(
        2,
        |batch: Vec<i32>| std::format!("{batch:?}"),
        CollectSpout::<String>::new(),
    );

    let _ = s.send(1);
    let _ = s.send(2);
    let _ = s.send(3);
    let _ = s.send(4);
    let _ = s.flush();

    assert_eq!(
        s.into_inner().into_items(),
        vec!["[1, 2]".to_string(), "[3, 4]".to_string()]
    );
}

#[test]
fn reduce_spout_accessors() {
    let s: ReduceSpout<i32, usize, _, CollectSpout<usize>> =
        ReduceSpout::new(10, |b: Vec<i32>| b.len(), CollectSpout::new());
    assert_eq!(s.threshold(), 10);
    assert_eq!(s.buffered(), 0);
}

// --- send_all tests ---

#[test]
fn send_all_default_delegates_to_send() {
    let mut s = CollectSpout::new();
    let _ = s.send_all([1, 2, 3, 4, 5].into_iter());
    assert_eq!(s.items(), vec![1, 2, 3, 4, 5]);
}

#[test]
fn send_all_empty_iterator() {
    let mut s = CollectSpout::<i32>::new();
    let _ = s.send_all(core::iter::empty());
    assert!(s.items().is_empty());
}

#[test]
fn send_all_with_batch_spout_triggers_threshold() {
    let mut s: BatchSpout<i32, CollectSpout<Vec<i32>>> = BatchSpout::new(3, CollectSpout::new());
    let _ = s.send_all([1, 2, 3, 4, 5].into_iter());

    // First 3 should have triggered a batch
    assert_eq!(s.inner().items(), vec![vec![1, 2, 3]]);
    assert_eq!(s.buffered(), 2);
}

// --- Edge case tests ---

#[test]
fn batch_spout_single_item_flush() {
    let mut s: BatchSpout<i32, CollectSpout<Vec<i32>>> = BatchSpout::new(100, CollectSpout::new());
    let _ = s.send(42);
    let _ = s.flush();
    assert_eq!(s.into_inner().into_items(), vec![vec![42]]);
}

#[test]
fn batch_spout_threshold_of_one() {
    let mut s: BatchSpout<i32, CollectSpout<Vec<i32>>> = BatchSpout::new(1, CollectSpout::new());
    let _ = s.send(1);
    let _ = s.send(2);
    let _ = s.send(3);
    assert_eq!(s.inner().items(), vec![vec![1], vec![2], vec![3]]);
}

#[test]
fn batch_spout_large_batch() {
    let mut s: BatchSpout<i32, CollectSpout<Vec<i32>>> = BatchSpout::new(1000, CollectSpout::new());
    for i in 0..2500 {
        let _ = s.send(i);
    }
    let _ = s.flush();

    let batches = s.into_inner().into_items();
    assert_eq!(batches.len(), 3); // 1000 + 1000 + 500
    assert_eq!(batches[0].len(), 1000);
    assert_eq!(batches[1].len(), 1000);
    assert_eq!(batches[2].len(), 500);
}

#[test]
fn reduce_spout_empty_flush_is_noop() {
    let mut s = ReduceSpout::new(10, |b: Vec<i32>| b.len(), CollectSpout::new());
    let _ = s.flush();
    assert!(s.into_inner().into_items().is_empty());
}

#[test]
fn reduce_spout_threshold_of_one() {
    let mut s = ReduceSpout::new(1, |batch: Vec<i32>| batch[0] * 10, CollectSpout::new());
    let _ = s.send(1);
    let _ = s.send(2);
    let _ = s.send(3);
    assert_eq!(s.into_inner().into_items(), vec![10, 20, 30]);
}

#[test]
#[should_panic(expected = "BatchSpout threshold must be at least 1")]
fn batch_spout_rejects_zero_threshold() {
    let _: BatchSpout<i32, CollectSpout<Vec<i32>>> = BatchSpout::new(0, CollectSpout::new());
}

#[test]
#[should_panic(expected = "ReduceSpout threshold must be at least 1")]
fn reduce_spout_rejects_zero_threshold() {
    let _: ReduceSpout<i32, usize, _, CollectSpout<usize>> =
        ReduceSpout::new(0, |b: Vec<i32>| b.len(), CollectSpout::<usize>::new());
}

#[test]
fn batch_spout_send_after_flush() {
    let mut s: BatchSpout<i32, CollectSpout<Vec<i32>>> = BatchSpout::new(3, CollectSpout::new());
    let _ = s.send(1);
    let _ = s.send(2);
    let _ = s.flush();
    // Buffer should be usable after flush
    let _ = s.send(3);
    let _ = s.send(4);
    let _ = s.send(5);
    // One batch from flush (partial), one from threshold
    assert_eq!(s.inner().items(), vec![vec![1, 2], vec![3, 4, 5]]);
}

#[test]
fn collect_spout_take_leaves_empty() {
    let mut s = CollectSpout::new();
    let _ = s.send(1);
    let _ = s.send(2);
    let taken = s.take();
    assert_eq!(taken, vec![1, 2]);
    assert!(s.items().is_empty());

    // Can continue sending after take
    let _ = s.send(3);
    assert_eq!(s.items(), vec![3]);
}
