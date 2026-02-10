//! SpillRing: single-threaded ring buffer with overflow to spout.
//!
//! When the ring is full, the oldest item is evicted to the configured
//! spout. Default spout drops evicted items; use a custom spout to
//! collect, log, or forward them.
//!
//! Run with: cargo run --example spill_ring

use spill_ring::SpillRing;
use spout::CollectSpout;

fn main() {
    // A 4-slot ring that collects evicted items into a Vec.
    let sink = CollectSpout::<u64>::new();
    let mut ring = SpillRing::<u64, 4>::builder().sink(sink).build();

    // Push 8 items — the first 4 will be evicted to the spout.
    for i in 0..8 {
        ring.push_mut(i);
    }

    println!("Ring contents (newest 4):");
    for val in ring.iter() {
        print!("  {val}");
    }
    println!();

    println!("Evicted to spout (oldest 4):");
    for val in ring.sink().items() {
        print!("  {val}");
    }
    println!();

    // Flush remaining items to spout and drain everything.
    ring.flush();
    println!(
        "After flush, spout has {} items total",
        ring.sink().items().len()
    );
}
