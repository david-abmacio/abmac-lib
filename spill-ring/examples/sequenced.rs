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

    // streaming_fan_in returns a guard — no unsafe needed.
    let mut stream = pool.streaming_fan_in(SequencedCollector::from_spout(CollectSpout::new()));

    for round in 0..ROUNDS {
        stream.dispatch(&round);
        stream.join().unwrap();
        stream.flush().unwrap();
    }

    let items = stream.into_collector().into_inner_spout().into_items();

    println!("Sequenced {} items across {ROUNDS} rounds:", items.len());
    for item in &items {
        print!("  {item}");
    }
    println!();
}
