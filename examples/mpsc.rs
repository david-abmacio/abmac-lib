//! MPSC example: Multiple producers write sensor data, merged and flushed to file.
//!
//! Demonstrates zero-overhead MPSC with file I/O and checksum verification.
//! Each producer runs independently at full speed, data flushes to shared sink.
//!
//! Run with: cargo run --example mpsc

use spill_ring::{MpscRing, Sink};
use std::{
    fs::File,
    hash::{DefaultHasher, Hash, Hasher},
    io::{BufWriter, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
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

    fn hash_into(&self, hasher: &mut DefaultHasher) {
        self.timestamp.hash(hasher);
        self.sensor_id.hash(hasher);
        hasher.write(&self.value.to_le_bytes());
    }
}

/// Shared sink state protected by mutex.
struct SharedSinkInner {
    writer: BufWriter<File>,
    hashers: Vec<DefaultHasher>, // Per-producer hashers
    count: u64,
}

/// Thread-safe sink for evictions from all producers.
#[derive(Clone)]
struct SharedFileSink {
    inner: Arc<Mutex<SharedSinkInner>>,
}

impl SharedFileSink {
    fn new(path: &str, num_producers: usize) -> std::io::Result<Self> {
        let file = File::create(path)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(SharedSinkInner {
                writer: BufWriter::new(file),
                hashers: (0..num_producers).map(|_| DefaultHasher::new()).collect(),
                count: 0,
            })),
        })
    }

    fn count(&self) -> u64 {
        self.inner.lock().unwrap().count
    }

    fn get_checksums(&self) -> Vec<u64> {
        self.inner
            .lock()
            .unwrap()
            .hashers
            .iter()
            .map(|h| h.clone().finish())
            .collect()
    }

    fn flush_inner(&self) {
        self.inner.lock().unwrap().writer.flush().unwrap();
    }
}

impl Sink<SensorReading> for SharedFileSink {
    fn send(&mut self, item: SensorReading) {
        let mut inner = self.inner.lock().unwrap();
        let producer_id = item.sensor_id as usize;
        item.hash_into(&mut inner.hashers[producer_id]);
        inner.writer.write_all(&item.to_bytes()).unwrap();
        inner.count += 1;
    }

    fn flush(&mut self) {
        self.inner.lock().unwrap().writer.flush().unwrap();
    }
}

fn main() -> std::io::Result<()> {
    const NUM_PRODUCERS: usize = 4;
    const READINGS_PER_PRODUCER: u64 = 250_000;
    const TOTAL_READINGS: u64 = NUM_PRODUCERS as u64 * READINGS_PER_PRODUCER;
    const RING_CAPACITY: usize = 1024;

    println!("MPSC File Sink Example");
    println!("======================");
    println!("Producers: {}", NUM_PRODUCERS);
    println!("Readings per producer: {}", READINGS_PER_PRODUCER);
    println!("Total readings: {}", TOTAL_READINGS);
    println!(
        "Ring capacity per producer: {} (will spill to shared sink)",
        RING_CAPACITY
    );
    println!();

    // Create shared sink for evictions - all producers write to same file
    let sink = SharedFileSink::new("mpsc_output.bin", NUM_PRODUCERS)?;
    let sink_ref = sink.clone();

    // Create MPSC ring - each producer gets a clone of the sink
    let producers = MpscRing::<SensorReading, RING_CAPACITY, _>::with_sink(NUM_PRODUCERS, sink);

    // Track input checksums per-producer (order within producer is preserved)
    let input_checksums: Vec<Arc<AtomicU64>> = (0..NUM_PRODUCERS)
        .map(|_| Arc::new(AtomicU64::new(0)))
        .collect();

    // Spawn producers - each runs at full no-atomics speed
    // Items flush to shared sink on overflow and when producer drops
    thread::scope(|s| {
        for (producer_id, producer) in producers.into_iter().enumerate() {
            let checksum_slot = Arc::clone(&input_checksums[producer_id]);
            s.spawn(move || {
                let mut hasher = DefaultHasher::new();

                for i in 0..READINGS_PER_PRODUCER {
                    let reading = SensorReading {
                        timestamp: i,
                        sensor_id: producer_id as u32,
                        value: (i as f64 + producer_id as f64).sin(),
                    };
                    reading.hash_into(&mut hasher);
                    producer.push(reading);
                }

                // Store this producer's checksum
                checksum_slot.store(hasher.finish(), Ordering::Relaxed);
                // Producer drops here, remaining items flush to sink
            });
        }
    });

    sink_ref.flush_inner();

    println!("Results:");
    println!("  Items generated:  {}", TOTAL_READINGS);
    println!("  Items to file:    {}", sink_ref.count());
    println!();

    // Verify per-producer checksums (order within each producer preserved)
    println!("Per-producer checksum verification:");
    let output_checksums = sink_ref.get_checksums();
    let mut all_match = true;
    for i in 0..NUM_PRODUCERS {
        let input = input_checksums[i].load(Ordering::Relaxed);
        let output = output_checksums[i];
        let status = if input == output { "PASS" } else { "FAIL" };
        if input != output {
            all_match = false;
        }
        println!(
            "  Producer {}: input={:016x} output={:016x} [{}]",
            i, input, output, status
        );
    }

    println!();
    if all_match {
        println!("Overall Status: PASS - all producer checksums match!");
    } else {
        println!("Overall Status: FAIL - checksum mismatch detected!");
        std::process::exit(1);
    }

    // Cleanup
    std::fs::remove_file("mpsc_output.bin")?;

    Ok(())
}
