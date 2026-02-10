extern crate std;

use std::sync::Arc;
use std::thread;

use crate::SpscRing;

/// Test SPSC: one producer thread, one consumer thread.
#[test]
fn spsc_producer_consumer() {
    let ring = Arc::new(SpscRing::<usize, 64>::new());
    let num_items: usize = 10_000;

    let producer_ring = Arc::clone(&ring);
    let producer = thread::spawn(move || {
        for i in 0..num_items {
            producer_ring.push(i);
        }
    });

    let consumer_ring = Arc::clone(&ring);
    let consumer = thread::spawn(move || {
        let mut received = std::vec::Vec::new();
        let mut last_value: Option<usize> = None;
        let mut spins = 0;

        loop {
            if let Some(v) = consumer_ring.pop() {
                // Verify monotonic ordering (SPSC guarantees this)
                if let Some(last) = last_value {
                    assert!(v > last, "values not monotonic: {} then {}", last, v);
                }
                last_value = Some(v);
                received.push(v);
                spins = 0;
            } else {
                spins += 1;
                // Give up after many empty spins (producer done, items evicted)
                if spins > 100_000 {
                    break;
                }
                thread::yield_now();
            }
        }
        received
    });

    producer.join().expect("producer panicked");
    let received = consumer.join().expect("consumer panicked");

    // Consumer should have received some items (not all due to eviction)
    assert!(!received.is_empty());
    // All received values should be valid (in expected range)
    for &v in &received {
        assert!(v < num_items, "invalid value received: {}", v);
    }
}

/// Stress test: rapid push/pop in SPSC pattern.
#[test]
fn spsc_stress() {
    let ring = Arc::new(SpscRing::<usize, 16>::new());
    let iterations = 50_000;

    let producer_ring = Arc::clone(&ring);
    let producer = thread::spawn(move || {
        for i in 0..iterations {
            producer_ring.push(i);
        }
    });

    let consumer_ring = Arc::clone(&ring);
    let consumer = thread::spawn(move || {
        let mut count = 0;
        let mut last: Option<usize> = None;
        let mut spins = 0;
        loop {
            if let Some(v) = consumer_ring.pop() {
                // Verify ordering
                if let Some(l) = last {
                    assert!(v > l, "ordering violated: {} then {}", l, v);
                }
                last = Some(v);
                count += 1;
                spins = 0;
            } else {
                spins += 1;
                if spins > 100_000 {
                    break;
                }
            }
        }
        count
    });

    producer.join().expect("producer panicked");
    let consumed = consumer.join().expect("consumer panicked");

    // Should have consumed some items
    assert!(consumed > 0);
}

/// Test split() — typed producer/consumer handles enforce SPSC contract.
#[test]
fn spsc_split_basic() {
    let (producer, consumer) = SpscRing::<usize, 64>::new().split();
    let num_items: usize = 10_000;

    let t = thread::spawn(move || {
        for i in 0..num_items {
            producer.push(i);
        }
    });

    t.join().expect("producer panicked");

    // Drain remaining items — verify monotonic ordering
    let mut received = std::vec::Vec::new();
    while let Some(v) = consumer.pop() {
        received.push(v);
    }

    assert!(!received.is_empty());
    for w in received.windows(2) {
        assert!(w[1] > w[0], "values not monotonic: {} then {}", w[0], w[1]);
    }
}

/// Test split() with concurrent producer and consumer.
#[test]
fn spsc_split_concurrent() {
    let (producer, consumer) = SpscRing::<usize, 32>::new().split();
    let num_items: usize = 50_000;

    let prod_handle = thread::spawn(move || {
        for i in 0..num_items {
            producer.push(i);
        }
    });

    let cons_handle = thread::spawn(move || {
        let mut count = 0;
        let mut last: Option<usize> = None;
        let mut spins = 0;
        loop {
            if let Some(v) = consumer.pop() {
                if let Some(l) = last {
                    assert!(v > l, "ordering violated: {} then {}", l, v);
                }
                last = Some(v);
                count += 1;
                spins = 0;
            } else {
                spins += 1;
                if spins > 100_000 {
                    break;
                }
            }
        }
        count
    });

    prod_handle.join().expect("producer panicked");
    let consumed = cons_handle.join().expect("consumer panicked");
    assert!(consumed > 0);
}

/// Test that consumer sees consistent state during producer activity.
#[test]
fn spsc_len_consistency() {
    let ring = Arc::new(SpscRing::<usize, 32>::new());

    let producer_ring = Arc::clone(&ring);
    let producer = thread::spawn(move || {
        for i in 0..5000 {
            producer_ring.push(i);
            if i % 100 == 0 {
                thread::yield_now();
            }
        }
    });

    let consumer_ring = Arc::clone(&ring);
    let consumer = thread::spawn(move || {
        for _ in 0..1000 {
            let len = consumer_ring.len();
            // len should never exceed capacity
            assert!(len <= 32, "len {} exceeds capacity", len);
            thread::yield_now();
        }
    });

    producer.join().expect("producer panicked");
    consumer.join().expect("consumer panicked");
}
