//! SPSC example: Single producer writes sensor data, sink flushes to file.
//!
//! Demonstrates the spill pattern with file I/O and checksum verification.
//!
//! Run with: cargo run --example spsc_file_sink

use spill_ring::SpillRing;
use spout::FnFlushSink;
use std::{
    cell::RefCell,
    fs::File,
    hash::{DefaultHasher, Hash, Hasher},
    io::{BufWriter, Write},
    rc::Rc,
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

    fn hash_into(&self, hasher: &mut DefaultHasher) {
        self.timestamp.hash(hasher);
        self.sensor_id.hash(hasher);
        hasher.write(&self.value.to_le_bytes());
    }
}

/// Shared state for tracking output.
struct OutputState {
    writer: BufWriter<File>,
    hasher: DefaultHasher,
    count: u64,
}

fn main() -> std::io::Result<()> {
    const NUM_READINGS: u64 = 1_000_000;
    const RING_CAPACITY: usize = 1024;

    println!("SPSC File Sink Example");
    println!("======================");
    println!("Generating {} sensor readings...", NUM_READINGS);
    println!("Ring capacity: {} (will spill to file)", RING_CAPACITY);
    println!();

    // Shared state for the sink
    let state = Rc::new(RefCell::new(OutputState {
        writer: BufWriter::new(File::create("spsc_output.bin")?),
        hasher: DefaultHasher::new(),
        count: 0,
    }));

    // Create sink closures that capture shared state
    let state_send = Rc::clone(&state);
    let state_flush = Rc::clone(&state);

    let sink = FnFlushSink::new(
        move |item: SensorReading| {
            let mut s = state_send.borrow_mut();
            item.hash_into(&mut s.hasher);
            s.writer.write_all(&item.to_bytes()).unwrap();
            s.count += 1;
        },
        move || {
            state_flush.borrow_mut().writer.flush().unwrap();
        },
    );

    let mut ring: SpillRing<SensorReading, RING_CAPACITY, _> = SpillRing::with_sink(sink);

    // Compute input checksum while generating data
    let mut input_hasher = DefaultHasher::new();

    // Generate and push sensor readings
    for i in 0..NUM_READINGS {
        let reading = SensorReading {
            timestamp: i,
            sensor_id: (i % 10) as u32,
            value: (i as f64).sin(),
        };
        reading.hash_into(&mut input_hasher);
        ring.push(reading);
    }

    // Flush remaining items in ring to sink
    ring.flush();

    let input_checksum = input_hasher.finish();
    let s = state.borrow();
    let output_checksum = s.hasher.clone().finish();
    let output_count = s.count;
    drop(s);

    println!("Results:");
    println!("  Items generated:  {}", NUM_READINGS);
    println!("  Items to file:    {}", output_count);
    println!();
    println!("Checksum verification:");
    println!("  Input checksum:   {:016x}", input_checksum);
    println!("  Output checksum:  {:016x}", output_checksum);

    if input_checksum == output_checksum {
        println!("  Status:           PASS - checksums match!");
    } else {
        println!("  Status:           FAIL - checksum mismatch!");
        std::process::exit(1);
    }

    // Cleanup
    std::fs::remove_file("spsc_output.bin")?;

    Ok(())
}
