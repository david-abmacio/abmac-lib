extern crate std;

use std::vec;

use crate::SpillRing;
use spout::{CollectSpout, FnSpout};

#[test]
fn ring_chaining_basic() {
    // ring1 overflows into ring2
    let ring2: SpillRing<i32, 4> = SpillRing::new();
    let ring1 = SpillRing::<i32, 2, _>::with_sink(ring2);

    ring1.push(1);
    ring1.push(2);
    // ring1 full: [1, 2], ring2 empty

    ring1.push(3); // evicts 1 to ring2
    ring1.push(4); // evicts 2 to ring2
    // ring1: [3, 4], ring2: [1, 2]

    assert_eq!(ring1.pop(), Some(3));
    assert_eq!(ring1.pop(), Some(4));

    // Access ring2 via sink
    assert_eq!(ring1.sink().pop(), Some(1));
    assert_eq!(ring1.sink().pop(), Some(2));
}

#[test]
fn ring_chaining_cascade_overflow() {
    // ring1 -> ring2 -> CollectSpout
    // When ring2 also overflows, items go to final sink
    let final_sink = CollectSpout::new();
    let ring2 = SpillRing::<i32, 2, _>::with_sink(final_sink);
    let ring1 = SpillRing::<i32, 2, _>::with_sink(ring2);

    // Push 6 items through ring1 (cap 2) -> ring2 (cap 2) -> final_sink
    for i in 1..=6 {
        ring1.push(i);
    }

    // ring1: [5, 6] (most recent)
    // ring2: [3, 4] (evicted from ring1, overflow of 1,2 went to final_sink)
    // final_sink: [1, 2]

    assert_eq!(ring1.sink().sink().items(), vec![1, 2]);
    assert_eq!(ring1.sink().pop(), Some(3));
    assert_eq!(ring1.sink().pop(), Some(4));
    assert_eq!(ring1.pop(), Some(5));
    assert_eq!(ring1.pop(), Some(6));
}

#[test]
fn ring_chaining_flush_cascades() {
    let final_sink = CollectSpout::new();
    let ring2 = SpillRing::<i32, 4, _>::with_sink(final_sink);
    let mut ring1 = SpillRing::<i32, 4, _>::with_sink(ring2);

    ring1.push(1);
    ring1.push(2);
    ring1.push(3);

    // Flush ring1 -> items go to ring2
    ring1.flush();
    assert!(ring1.is_empty());
    assert_eq!(ring1.sink().len(), 3);

    // Flush ring2 -> items go to final_sink
    ring1.sink_mut().flush();
    assert!(ring1.sink().is_empty());
    assert_eq!(ring1.sink().sink().items(), vec![1, 2, 3]);
}

#[test]
fn ring_chaining_drop_flushes_all() {
    use std::sync::{Arc, Mutex};

    let collected = Arc::new(Mutex::new(std::vec::Vec::new()));
    let collected_clone = collected.clone();

    {
        let final_sink = FnSpout(move |x: i32| {
            collected_clone.lock().unwrap().push(x);
        });
        let ring2 = SpillRing::<i32, 4, _>::with_sink(final_sink);
        let ring1 = SpillRing::<i32, 4, _>::with_sink(ring2);

        ring1.push(10);
        ring1.push(20);
        ring1.push(30);
        // ring1 dropped here -> flushes to ring2 -> ring2 dropped -> flushes to final_sink
    }

    assert_eq!(*collected.lock().unwrap(), vec![10, 20, 30]);
}
