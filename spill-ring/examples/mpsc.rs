//! MPSC ring: multiple producers, single consumer (alloc-only API).
//!
//! Each producer owns its own SpillRing — zero contention. After
//! producers finish, their rings are reunited with the consumer
//! and drained. No threads required.
//!
//! This is the alloc-level API. For threaded workloads, use WorkerPool
//! (see the worker_pool example).
//!
//! Run with: cargo run --example mpsc --features alloc

use spill_ring::MpscRing;
use spout::CollectSpout;

fn main() {
    const NUM_PRODUCERS: usize = 4;

    let (producers, mut consumer) = MpscRing::<u64, 8>::with_consumer(NUM_PRODUCERS);

    // Each producer fills its own ring independently.
    let finished: Vec<_> = producers
        .into_iter()
        .enumerate()
        .map(|(id, producer)| {
            for i in 0..20 {
                producer.push(id as u64 * 100 + i);
            }
            producer
        })
        .collect();

    // Reunite producers with consumer, then drain.
    consumer.collect(finished);

    let mut spout = CollectSpout::new();
    consumer.drain(&mut spout);
    let items = spout.into_items();

    // 8-slot rings hold at most 8 items each. With 20 pushes per producer,
    // the oldest 12 per producer were evicted (dropped by DropSpout).
    println!("Producers: {NUM_PRODUCERS}, ring capacity: 8");
    println!("Pushed 20 items each, drained {} total", items.len());
}
