extern crate std;

use crate::MpscRing;
use spout::{CollectSpout, ProducerSpout};

#[test]
fn basic_mpsc() {
    let (producers, mut consumer) = MpscRing::<u64, 8>::with_consumer(2);

    // Producer 0
    producers[0].push(1);
    producers[0].push(2);

    // Producer 1
    producers[1].push(10);
    producers[1].push(20);

    assert_eq!(producers[0].len(), 2);
    assert_eq!(producers[1].len(), 2);

    // Collect producers into consumer, then drain
    consumer.collect(producers);
    let mut spout = CollectSpout::new();
    consumer.drain(&mut spout);

    let mut items = spout.into_items();
    items.sort_unstable();
    assert_eq!(items, std::vec![1, 2, 10, 20]);
}

#[test]
fn producer_overflow_to_spout() {
    let spout = ProducerSpout::new(|_id| CollectSpout::<u64>::new());
    let producers = MpscRing::<u64, 4, _>::with_spout(1, spout);
    let producer = producers.into_iter().next().unwrap();

    // Overflow — push 10 items into ring of size 4
    for i in 0..10 {
        producer.push(i);
    }

    // Ring should have last 4 items, first 6 evicted to spout
    assert_eq!(producer.len(), 4);

    let ring = producer.into_ring();
    let evicted = ring.spout().inner().unwrap().items();
    assert_eq!(evicted, &[0, 1, 2, 3, 4, 5]);
    assert_eq!(ring.pop(), Some(6));
    assert_eq!(ring.pop(), Some(7));
    assert_eq!(ring.pop(), Some(8));
    assert_eq!(ring.pop(), Some(9));
}

#[test]
fn single_producer() {
    let (producers, mut consumer) = MpscRing::<u64, 16>::with_consumer(1);

    let producer = producers.into_iter().next().unwrap();
    for i in 0..10 {
        producer.push(i);
    }

    assert_eq!(producer.len(), 10);

    consumer.collect(std::vec![producer]);
    let mut spout = CollectSpout::new();
    consumer.drain(&mut spout);

    assert_eq!(spout.into_items(), std::vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
}

#[test]
fn with_consumer_and_collect() {
    let (producers, mut consumer) = MpscRing::<u64, 16>::with_consumer(3);

    producers[0].push(1);
    producers[0].push(2);
    producers[1].push(10);
    producers[2].push(100);

    consumer.collect(producers);

    assert_eq!(consumer.num_producers(), 3);
    assert_eq!(consumer.len(), 4);
    assert!(!consumer.is_empty());

    let mut spout = CollectSpout::new();
    consumer.drain(&mut spout);

    let mut items = spout.into_items();
    items.sort_unstable();
    assert_eq!(items, std::vec![1, 2, 10, 100]);

    assert!(consumer.is_empty());
    assert_eq!(consumer.len(), 0);
}
