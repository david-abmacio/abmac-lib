use crate::{CollectSpout, SequencedSpout};

#[test]
fn fast_path_in_order() {
    // All batches arrive in order — zero buffering.
    let mut seq = SequencedSpout::new(CollectSpout::new());
    seq.submit(0, [1, 2, 3].into_iter()).unwrap();
    seq.advance().unwrap();
    seq.submit(1, [4, 5].into_iter()).unwrap();
    seq.advance().unwrap();
    seq.submit(2, [6].into_iter()).unwrap();
    seq.advance().unwrap();
    assert_eq!(seq.inner().items(), &[1, 2, 3, 4, 5, 6]);
    assert_eq!(seq.buffered_batches(), 0);
    assert_eq!(seq.next_expected(), 3);
}

#[test]
fn reorder_two_batches() {
    // Batch 1 arrives before batch 0.
    let mut seq = SequencedSpout::new(CollectSpout::new());
    seq.submit(1, [4, 5].into_iter()).unwrap();
    assert!(seq.inner().items().is_empty()); // buffered
    assert_eq!(seq.buffered_batches(), 1);

    seq.submit(0, [1, 2, 3].into_iter()).unwrap();
    seq.advance().unwrap();
    // Batch 0 emitted, advance drained batch 1 too.
    assert_eq!(seq.inner().items(), &[1, 2, 3, 4, 5]);
    assert_eq!(seq.buffered_batches(), 0);
}

#[test]
fn gap_then_fill() {
    // Batch 2 arrives, then 0, then 1 — all three emit after advance past 1.
    let mut seq = SequencedSpout::new(CollectSpout::new());
    seq.submit(2, [7].into_iter()).unwrap();
    seq.submit(0, [1, 2].into_iter()).unwrap();
    seq.advance().unwrap();
    assert_eq!(seq.inner().items(), &[1, 2]); // only batch 0
    assert_eq!(seq.buffered_batches(), 1); // batch 2 still buffered

    seq.submit(1, [4, 5].into_iter()).unwrap();
    seq.advance().unwrap();
    assert_eq!(seq.inner().items(), &[1, 2, 4, 5, 7]); // batch 1 + batch 2
    assert_eq!(seq.buffered_batches(), 0);
}

#[test]
fn custom_start_seq() {
    let mut seq = SequencedSpout::with_start_seq(CollectSpout::new(), 10);
    assert_eq!(seq.next_expected(), 10);

    seq.submit(10, [1].into_iter()).unwrap();
    seq.advance().unwrap();
    assert_eq!(seq.inner().items(), &[1]);
    assert_eq!(seq.next_expected(), 11);
}

#[test]
fn into_inner_drops_buffered() {
    let mut seq = SequencedSpout::new(CollectSpout::new());
    seq.submit(0, [1].into_iter()).unwrap();
    seq.advance().unwrap();
    seq.submit(2, [3].into_iter()).unwrap(); // buffered, will be dropped
    assert_eq!(seq.buffered_batches(), 1);

    let inner = seq.into_inner();
    assert_eq!(inner.items(), &[1]); // only in-order batch emitted
}

#[test]
fn empty_batches() {
    let mut seq = SequencedSpout::new(CollectSpout::<u64>::new());
    seq.submit(0, core::iter::empty()).unwrap();
    seq.advance().unwrap();
    seq.submit(1, core::iter::empty()).unwrap();
    seq.advance().unwrap();
    assert!(seq.inner().items().is_empty());
    assert_eq!(seq.next_expected(), 2);
}

#[test]
fn multiple_workers_same_seq() {
    // N workers share the same seq — all emit on the fast path.
    let mut seq = SequencedSpout::new(CollectSpout::new());
    seq.submit(0, [1].into_iter()).unwrap();
    seq.submit(0, [2].into_iter()).unwrap();
    seq.submit(0, [3].into_iter()).unwrap();
    seq.advance().unwrap();
    assert_eq!(seq.inner().items(), &[1, 2, 3]);
    assert_eq!(seq.next_expected(), 1);
    assert_eq!(seq.buffered_batches(), 0);
}

#[test]
fn multiple_workers_same_seq_out_of_order() {
    // Workers for seq 1 arrive before seq 0 workers finish.
    let mut seq = SequencedSpout::new(CollectSpout::new());
    // Seq 1 arrives first (2 workers)
    seq.submit(1, [10].into_iter()).unwrap();
    seq.submit(1, [11].into_iter()).unwrap();
    // Seq 0 arrives (2 workers)
    seq.submit(0, [1].into_iter()).unwrap();
    seq.submit(0, [2].into_iter()).unwrap();
    seq.advance().unwrap(); // advance past seq 0, drains buffered seq 1
    assert_eq!(seq.inner().items(), &[1, 2, 10, 11]);
    assert_eq!(seq.buffered_batches(), 0);
    assert_eq!(seq.next_expected(), 2);
}
