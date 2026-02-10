//! SPSC ring: one producer thread, one consumer thread.
//!
//! Run with: cargo run --example spsc --features std

use spill_ring::SpscRing;
use std::thread;

fn main() {
    const ITEMS: u64 = 1_000_000;

    // Build and split into typed handles — the producer can only push,
    // the consumer can only pop. Neither is Clone (enforces single-producer,
    // single-consumer at compile time).
    let (producer, consumer) = SpscRing::<u64, 1024>::builder().build().split();

    let writer = thread::spawn(move || {
        for i in 0..ITEMS {
            producer.push(i);
        }
    });

    let reader = thread::spawn(move || {
        let mut received = 0u64;
        let mut spins = 0u32;
        loop {
            if let Some(_val) = consumer.pop() {
                received += 1;
                spins = 0;
            } else {
                spins += 1;
                if spins > 100_000 {
                    break;
                }
            }
        }
        received
    });

    writer.join().unwrap();
    let received = reader.join().unwrap();

    // Some items may be evicted (dropped) when the ring is full and
    // the consumer hasn't popped yet — that's the spill-ring contract.
    println!("Pushed {ITEMS}, consumer received {received}");
}
