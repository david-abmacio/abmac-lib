//! MPSM: multiple mergers drain workers in parallel.
//!
//! Each MergerHandle owns a subset of handoff slots (round-robin).
//! Merger threads run in parallel via thread::scope, each draining
//! its assigned workers independently.
//!
//! Run with: cargo run --example mergers --features std

use spill_ring::{MpscRing, UnorderedCollector};
use spout::CollectSpout;
use std::thread;

fn main() {
    const NUM_WORKERS: usize = 8;
    const NUM_MERGERS: usize = 2;

    let mut pool = MpscRing::<u64, 256>::pool(NUM_WORKERS).spawn(|ring, worker_id, count: &u64| {
        for i in 0..*count {
            ring.push(worker_id as u64 * 1000 + i);
        }
    });

    pool.run(&50);

    // Each merger gets its own CollectSpout via the factory.
    // Merger 0 drains workers {0, 2, 4, 6}, merger 1 drains {1, 3, 5, 7}.
    let totals = pool.with_mergers(
        NUM_MERGERS,
        |_| UnorderedCollector::new(CollectSpout::new()),
        |mergers| {
            thread::scope(|s| {
                let handles: Vec<_> = mergers
                    .iter_mut()
                    .map(|m| {
                        s.spawn(|| {
                            m.flush().unwrap();
                            (m.merger_id(), m.collector().inner().items().len())
                        })
                    })
                    .collect();

                handles
                    .into_iter()
                    .map(|h| h.join().unwrap())
                    .collect::<Vec<_>>()
            })
        },
    );

    let grand_total: usize = totals.iter().map(|(_, count)| count).sum();
    for (id, count) in &totals {
        println!("Merger {id}: {count} items");
    }
    println!("Total: {grand_total} items from {NUM_WORKERS} workers");
}
