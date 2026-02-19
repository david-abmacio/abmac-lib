mod pool;
mod streaming;

use crate::MpscRing;
use spout::CollectSpout;

#[test]
fn multi_threaded_producers() {
    use std::thread;

    let (producers, mut consumer) = MpscRing::<u64, 64>::with_consumer(4);

    let finished: std::vec::Vec<_> = thread::scope(|s| {
        producers
            .into_iter()
            .enumerate()
            .map(|(id, producer)| {
                s.spawn(move || {
                    for i in 0..100 {
                        producer.push(id as u64 * 1000 + i);
                    }
                    producer
                })
            })
            .collect::<std::vec::Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect()
    });

    consumer.collect(finished);

    let mut spout = CollectSpout::new();
    consumer.drain(&mut spout);

    let items = spout.into_items();
    // 4 threads x 100 items, ring capacity 64 so last 64 per thread survive
    assert_eq!(items.len(), 256);

    // Verify each thread's items are the last 64 (36..100)
    for thread_id in 0..4u64 {
        let thread_items: std::vec::Vec<u64> = items
            .iter()
            .filter(|&&x| x / 1000 == thread_id)
            .copied()
            .collect();
        assert_eq!(thread_items.len(), 64);
        let expected: std::vec::Vec<u64> = (36..100).map(|i| thread_id * 1000 + i).collect();
        assert_eq!(thread_items, expected);
    }
}
