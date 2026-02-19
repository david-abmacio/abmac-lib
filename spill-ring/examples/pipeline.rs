//! Multi-stage pipeline: two WorkerPools chained via collect().
//!
//! Stage 1 produces raw values. Stage 2 squares them. Each stage
//! scales independently to its core allocation.
//!
//! Run with: cargo run --example pipeline --features std

use spill_ring::MpscRing;
use spout::CollectSpout;

fn main() {
    // Stage 1: generate values.
    let mut stage1 = MpscRing::<u64, 256>::pool(2).spawn(|ring, worker_id, count: &u64| {
        for i in 0..*count {
            ring.push(worker_id as u64 * 1000 + i);
        }
    });

    stage1.run(&10);

    // Collect stage 1 output.
    let mut between = CollectSpout::new();
    stage1.collect(&mut between).unwrap();
    let stage1_items = between.into_items();

    // Stage 2: square each value.
    let mut stage2 = MpscRing::<u64, 256>::pool(2).spawn(|ring, _worker_id, items: &Vec<u64>| {
        for &val in items {
            ring.push(val * val);
        }
    });

    stage2.run(&stage1_items);

    let mut out = CollectSpout::new();
    stage2.collect(&mut out).unwrap();

    println!(
        "Pipeline: {} items through stage 1, {} through stage 2",
        stage1_items.len(),
        out.items().len()
    );
}
