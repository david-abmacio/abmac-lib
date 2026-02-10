extern crate std;

use crate::SpillRing;
use crate::traits::{RingConsumer, RingInfo, RingProducer};

#[test]
fn ring_producer_trait() {
    let mut ring: SpillRing<i32, 4> = SpillRing::new();

    // try_push
    assert!(RingProducer::try_push(&mut ring, 1).is_ok());
    assert!(RingProducer::try_push(&mut ring, 2).is_ok());
    assert!(RingProducer::try_push(&mut ring, 3).is_ok());
    assert!(RingProducer::try_push(&mut ring, 4).is_ok());

    // is_full
    assert!(RingInfo::is_full(&ring));

    // try_push when full returns Err
    assert_eq!(RingProducer::try_push(&mut ring, 5), Err(5));

    // capacity, len, is_empty (from RingInfo)
    assert_eq!(RingInfo::capacity(&ring), 4);
    assert_eq!(RingInfo::len(&ring), 4);
    assert!(!RingInfo::is_empty(&ring));
}

#[test]
fn ring_consumer_trait() {
    let mut ring: SpillRing<i32, 4> = SpillRing::new();
    ring.push(10);
    ring.push(20);

    // peek
    assert_eq!(RingConsumer::peek(&mut ring), Some(&10));

    // try_pop
    assert_eq!(RingConsumer::try_pop(&mut ring), Some(10));
    assert_eq!(RingConsumer::try_pop(&mut ring), Some(20));
    assert_eq!(RingConsumer::try_pop(&mut ring), None);

    // is_empty, len, capacity (from RingInfo)
    assert!(RingInfo::is_empty(&ring));
    assert_eq!(RingInfo::len(&ring), 0);
    assert_eq!(RingInfo::capacity(&ring), 4);
}
