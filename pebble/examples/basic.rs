//! Basic usage of the pebble checkpoint manager.
//!
//! Run with: cargo run --example basic

use pebble::prelude::*;
use spout::DropSpout;

/// A simple checkpoint that stores a counter value.
#[derive(Clone, Debug)]
struct CounterCheckpoint {
    id: u64,
    value: u64,
}

impl Checkpointable for CounterCheckpoint {
    type Id = u64;
    type RebuildError = ();

    fn checkpoint_id(&self) -> u64 {
        self.id
    }

    fn compute_from_dependencies(
        base: Self,
        _deps: &hashbrown::HashMap<Self::Id, &Self>,
    ) -> core::result::Result<Self, Self::RebuildError> {
        Ok(base)
    }
}

/// Simple serializer using big-endian encoding.
struct CounterSerializer;

impl CheckpointSerializer<CounterCheckpoint> for CounterSerializer {
    type Error = &'static str;

    fn serialize(&self, cp: &CounterCheckpoint) -> core::result::Result<Vec<u8>, Self::Error> {
        let mut buf = Vec::with_capacity(16);
        buf.extend_from_slice(&cp.id.to_be_bytes());
        buf.extend_from_slice(&cp.value.to_be_bytes());
        Ok(buf)
    }

    fn deserialize(&self, bytes: &[u8]) -> core::result::Result<CounterCheckpoint, Self::Error> {
        if bytes.len() < 16 {
            return Err("buffer too small");
        }
        let id = u64::from_be_bytes(bytes[0..8].try_into().unwrap());
        let value = u64::from_be_bytes(bytes[8..16].try_into().unwrap());
        Ok(CounterCheckpoint { id, value })
    }
}

fn main() {
    // Create storage and manager with space for 4 checkpoints in fast memory
    let cold = DirectStorage::new(InMemoryStorage::<u64, u128, 8>::new(), CounterSerializer);
    let mut manager = PebbleManager::new(
        cold,
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        4,
    );

    println!("Adding 10 checkpoints (fast memory holds 4)...\n");

    // Add 10 checkpoints - older ones will be evicted to storage
    for i in 1..=10 {
        let checkpoint = CounterCheckpoint {
            id: i,
            value: i * 100,
        };
        manager.add(checkpoint, &[]).unwrap();
        println!("Added checkpoint {}: value = {}", i, i * 100);
    }

    println!("\n--- Checking locations ---\n");

    // Recent checkpoints are in fast memory
    for id in 1..=10 {
        let in_fast = manager.is_hot(id);
        let in_storage = manager.is_in_storage(id);
        let location = if in_fast {
            "fast memory (red pebble)"
        } else if in_storage {
            "storage (blue pebble)"
        } else {
            "not found"
        };
        println!("Checkpoint {}: {}", id, location);
    }

    println!("\n--- Accessing checkpoints ---\n");

    // Fast access to recent checkpoints (no I/O)
    if let Some(cp) = manager.get(10) {
        println!("Fast access to checkpoint 10: value = {}", cp.value);
    }

    // Load older checkpoint from storage (1 I/O operation)
    let cp = manager.load(1).unwrap();
    println!("Loaded checkpoint 1 from storage: value = {}", cp.value);

    // Now checkpoint 1 is in fast memory
    println!(
        "After load, checkpoint 1 in fast memory: {}",
        manager.is_hot(1)
    );

    println!("\n--- Statistics ---\n");

    let stats = manager.stats();
    println!("Checkpoints added: {}", stats.checkpoints_added());
    println!("Red pebbles (fast): {}", stats.red_pebble_count());
    println!("Blue pebbles (storage): {}", stats.blue_pebble_count());
    println!("I/O operations: {}", stats.io_operations());
}
