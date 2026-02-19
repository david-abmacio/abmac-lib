//! WorkerPool: thread-per-core pool with persistent threads.
//!
//! Each worker owns a pre-warmed SpillRing. The main thread dispatches
//! work, joins, and collects results. Zero contention on the produce path.
//!
//! Run with: cargo run --example worker_pool --features std

use spill_ring::MpscRing;
use spout::CollectSpout;

fn main() {
    const NUM_WORKERS: usize = 4;
    const ITEMS_PER_WORKER: u64 = 10_000;

    let mut pool =
        MpscRing::<u64, 1024>::pool(NUM_WORKERS).spawn(|ring, worker_id, count: &u64| {
            for i in 0..*count {
                ring.push(worker_id as u64 * 100_000 + i);
            }
        });

    // Run three rounds of work.
    let mut out = CollectSpout::new();
    for _ in 0..3 {
        pool.run(&ITEMS_PER_WORKER);
        pool.collect(&mut out).unwrap();
    }

    let consumer = pool.into_consumer();
    println!(
        "Collected {} items across 3 rounds with {} workers",
        out.items().len(),
        NUM_WORKERS
    );
    println!("Consumer holds {} remaining ring items", consumer.len());
}
