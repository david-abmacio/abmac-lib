//! MPSC ring: multiple producers, single consumer.
//!
//! Each producer owns its own SpillRing — zero contention on the hot path.
//! After producers finish, their rings are collected into a consumer and
//! drained.
//!
//! Run with: cargo run --example mpsc --features std

use spill_ring::{MpscRing, collect};
use spout::CollectSpout;
use std::thread;

fn main() {
    const NUM_PRODUCERS: usize = 4;
    const ITEMS_PER_PRODUCER: u64 = 100_000;

    // Create producers with a consumer handle for draining after work is done.
    let (producers, mut consumer) = MpscRing::<u64, 256>::with_consumer(NUM_PRODUCERS);

    // Spawn producers — each runs at full speed on its own ring.
    let finished: Vec<_> = thread::scope(|s| {
        producers
            .into_iter()
            .enumerate()
            .map(|(id, producer)| {
                s.spawn(move || {
                    for i in 0..ITEMS_PER_PRODUCER {
                        producer.push(id as u64 * ITEMS_PER_PRODUCER + i);
                    }
                    producer // return ownership so we can collect the ring
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect()
    });

    // Reunite producers with consumer, then drain remaining ring contents.
    collect(finished, &mut consumer);

    let mut sink = CollectSpout::new();
    consumer.drain(&mut sink);
    let items = sink.into_items();

    // Only the last 256 items per producer remain in each ring (earlier
    // items were dropped by DropSpout on overflow). With 256-slot rings
    // and 100k items, most items are evicted.
    println!("Producers: {NUM_PRODUCERS}, items each: {ITEMS_PER_PRODUCER}");
    println!("Drained from rings: {}", items.len());
}
