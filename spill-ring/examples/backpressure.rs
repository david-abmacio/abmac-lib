//! Lossless delivery with backpressure.
//!
//! A SpillRing absorbs bursts; overflow spills to a bounded SyncChannelSpout.
//! The producer only blocks when both the ring AND the channel are full,
//! meaning the consumer is genuinely falling behind.
//!
//! Run with: cargo run --example backpressure --features std

use spill_ring::SpillRing;
use spout::SyncChannelSpout;
use std::sync::mpsc;
use std::thread;

fn main() {
    const ITEMS: u64 = 1_000_000;

    // Bounded channel — blocks the producer when full.
    let (tx, rx) = mpsc::sync_channel::<u64>(1024);

    // Ring absorbs bursts; overflow goes to the channel.
    let mut ring = SpillRing::<u64, 256>::builder()
        .sink(SyncChannelSpout::new(tx))
        .build();

    let producer = thread::spawn(move || {
        for i in 0..ITEMS {
            ring.push(i);
        }
        ring.flush(); // flush remaining items to the channel
        drop(ring); // drop closes the sender
    });

    let consumer = thread::spawn(move || {
        let mut count = 0u64;
        while let Ok(_val) = rx.recv() {
            count += 1;
        }
        count
    });

    producer.join().unwrap();
    let received = consumer.join().unwrap();

    println!(
        "Sent {ITEMS}, received {received} — lossless: {}",
        received == ITEMS
    );
}
