//! MPSC zero-contention example: Each producer writes to its own file.
//!
//! Demonstrates `ProducerSink` for zero-contention MPSC patterns where each
//! producer gets its own independent sink.
//!
//! Run with: cargo run --example mpsc_zero_contention

use spill_ring::MpscRing;
use spout::{FnFlushSink, ProducerSink};
use std::{
    fs::File,
    io::{BufWriter, Write},
    thread,
};

/// A sensor reading with timestamp and value.
#[derive(Clone)]
struct SensorReading {
    timestamp: u64,
    sensor_id: u32,
    value: f64,
}

impl SensorReading {
    fn to_bytes(&self) -> [u8; 20] {
        let mut buf = [0u8; 20];
        buf[0..8].copy_from_slice(&self.timestamp.to_le_bytes());
        buf[8..12].copy_from_slice(&self.sensor_id.to_le_bytes());
        buf[12..20].copy_from_slice(&self.value.to_le_bytes());
        buf
    }
}

fn main() -> std::io::Result<()> {
    const NUM_PRODUCERS: usize = 4;
    const READINGS_PER_PRODUCER: u64 = 250_000;
    const TOTAL_READINGS: u64 = NUM_PRODUCERS as u64 * READINGS_PER_PRODUCER;
    const RING_CAPACITY: usize = 1024;

    println!("MPSC Zero-Contention Example");
    println!("============================");
    println!("Producers: {}", NUM_PRODUCERS);
    println!("Readings per producer: {}", READINGS_PER_PRODUCER);
    println!("Total readings: {}", TOTAL_READINGS);
    println!("Ring capacity per producer: {}", RING_CAPACITY);
    println!();
    println!("Each producer writes evictions to its own file - ZERO lock contention!");
    println!();

    // ProducerSink creates independent sinks via factory function.
    // Each clone gets a unique producer_id (0, 1, 2, ...).
    let sink = ProducerSink::new(|producer_id| {
        let path = format!("mpsc_producer_{}.bin", producer_id);
        let file = File::create(&path).expect("failed to create file");
        let mut writer = BufWriter::new(file);

        FnFlushSink::new(
            move |item: SensorReading| {
                writer.write_all(&item.to_bytes()).unwrap();
            },
            || {},
        )
    });

    // Create MPSC ring - each producer gets its own file sink
    let producers = MpscRing::<SensorReading, RING_CAPACITY, _>::with_sink(NUM_PRODUCERS, sink);

    // Spawn producers - each has its own file sink, zero contention
    // Items flush to per-producer files on overflow and when producer drops
    thread::scope(|s| {
        for (producer_id, producer) in producers.into_iter().enumerate() {
            s.spawn(move || {
                for i in 0..READINGS_PER_PRODUCER {
                    let reading = SensorReading {
                        timestamp: i,
                        sensor_id: producer_id as u32,
                        value: (i as f64 + producer_id as f64).sin(),
                    };
                    producer.push(reading);
                }
                // Producer drops here, remaining items flush to its file
            });
        }
    });

    println!("Results:");
    println!("  Items generated: {}", TOTAL_READINGS);
    println!();

    // Verify by reading back the per-producer files
    println!("Per-producer verification:");
    let mut all_match = true;
    let mut total_from_files = 0u64;

    for pid in 0..NUM_PRODUCERS {
        let path = format!("mpsc_producer_{}.bin", pid);
        let count = std::fs::read(&path)
            .map(|data| (data.len() / 20) as u64)
            .unwrap_or(0);

        total_from_files += count;

        let expected = READINGS_PER_PRODUCER;
        let status = if count == expected {
            "PASS"
        } else {
            all_match = false;
            "FAIL"
        };

        println!(
            "  Producer {}: {} items (expected {}) [{}]",
            pid, count, expected, status
        );

        // Cleanup producer file
        let _ = std::fs::remove_file(&path);
    }

    println!();
    println!(
        "  Total items: {} (expected {})",
        total_from_files, TOTAL_READINGS
    );

    if all_match && total_from_files == TOTAL_READINGS {
        println!();
        println!("Overall Status: PASS - all items accounted for!");
    } else {
        println!();
        println!("Overall Status: FAIL - item count mismatch!");
        std::process::exit(1);
    }

    Ok(())
}
