//! FanInSpout: composable single-consumer collection from a WorkerPool.
//!
//! Instead of collect(), FanInSpout drains handoff slots through the
//! Spout trait — enabling composition with batching, reduction, and
//! other spout wrappers.
//!
//! Run with: cargo run --example fan_in --features std

use spill_ring::MpscRing;

fn main() {
    const NUM_WORKERS: usize = 4;

    let mut pool = MpscRing::<u64, 256>::pool(NUM_WORKERS).spawn(|ring, worker_id, count: &u64| {
        for i in 0..*count {
            ring.push(worker_id as u64 * 1000 + i);
        }
    });

    pool.run(&100);

    // with_fan_in: scoped access to a FanInSpout that drains all slots.
    let count = pool.with_fan_in_collect(|fan_in| {
        fan_in.flush().unwrap();
        fan_in.collector().inner().items().len()
    });

    println!("Fan-in collected {count} items from {NUM_WORKERS} workers");
}
