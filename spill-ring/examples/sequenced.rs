//! Sequenced collection: ordered delivery from a WorkerPool.
//!
//! Workers complete in arbitrary order, but SequencedCollector reorders
//! batches so downstream receives items in dispatch sequence.
//!
//! Run with: cargo run --example sequenced --features std

use spill_ring::{MpscRing, SequencedCollector};
use spout::CollectSpout;

fn main() {
    const NUM_WORKERS: usize = 4;
    const ROUNDS: u64 = 5;

    let mut pool = MpscRing::<u64, 256>::pool(NUM_WORKERS).spawn(|ring, worker_id, round: &u64| {
        ring.push(*round * 100 + worker_id as u64);
    });

    // fan_in_unchecked returns an owned struct — no borrow on pool.
    // SAFETY: pool outlives fan_in — we drop fan_in before pool.
    let mut fan_in =
        unsafe { pool.fan_in_unchecked(SequencedCollector::from_spout(CollectSpout::new())) };

    for round in 0..ROUNDS {
        pool.dispatch(&round);
        pool.join().unwrap();
        fan_in.flush().unwrap();
    }

    let items = fan_in
        .into_collector()
        .into_sequencer()
        .into_inner()
        .into_items();

    drop(pool);

    println!("Sequenced {} items across {ROUNDS} rounds:", items.len());
    for item in &items {
        print!("  {item}");
    }
    println!();
}
