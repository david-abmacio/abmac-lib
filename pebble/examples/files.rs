//! Three-tier checkpoint manager flushing to files on disk.
//!
//! Demonstrates hot (red pebbles), warm, and cold tiers with a file-backed
//! storage backend. Watch checkpoint files appear in a temp directory as
//! items overflow from memory to disk.
//!
//! Run with:
//!   cargo run -p pebble --features cold-buffer,bytecast --example file_tiers

use std::collections::HashMap as StdHashMap;
use std::fs;
use std::path::{Path, PathBuf};

use bytecast::{DeriveFromBytes, DeriveToBytes};
use pebble::{CheckpointLoader, Checkpointable, PebbleManagerBuilder};
use spout::Spout;

#[derive(Clone, Debug, DeriveToBytes, DeriveFromBytes)]
struct Snapshot {
    id: u64,
    payload: Vec<u8>,
}

impl Checkpointable for Snapshot {
    type Id = u64;
    type RebuildError = ();

    fn checkpoint_id(&self) -> u64 {
        self.id
    }
    fn dependencies(&self) -> &[u64] {
        &[]
    }
    fn compute_from_dependencies(
        base: Self,
        _deps: &hashbrown::HashMap<u64, &Self>,
    ) -> Result<Self, ()> {
        Ok(base)
    }
}

/// Stores each checkpoint as a separate file: `<dir>/<id>.bin`.
struct FileStorage {
    dir: PathBuf,
    /// Track which IDs we've written (avoids scanning the filesystem).
    written: StdHashMap<u64, ()>,
}

impl FileStorage {
    fn new(dir: &Path) -> Self {
        fs::create_dir_all(dir).expect("create storage dir");
        Self {
            dir: dir.to_path_buf(),
            written: StdHashMap::new(),
        }
    }

    fn path_for(&self, id: u64) -> PathBuf {
        self.dir.join(format!("{id}.bin"))
    }
}

impl Spout<(u64, Vec<u8>)> for FileStorage {
    fn send(&mut self, (id, bytes): (u64, Vec<u8>)) {
        let path = self.path_for(id);
        fs::write(&path, &bytes).expect("write checkpoint file");
        self.written.insert(id, ());
        println!("  [disk] wrote {}", path.display());
    }
}

impl CheckpointLoader<u64> for FileStorage {
    fn load(&self, id: u64) -> Result<Vec<u8>, pebble::StorageError> {
        let path = self.path_for(id);
        fs::read(&path).map_err(|_| pebble::StorageError::NotFound)
    }

    fn contains(&self, id: u64) -> bool {
        self.written.contains_key(&id)
    }
}

fn main() {
    // Use a temp directory so we don't litter the repo.
    let dir = std::env::temp_dir().join("pebble_file_tiers_example");
    if dir.exists() {
        fs::remove_dir_all(&dir).ok();
    }

    // Manager lives in a block so Drop runs before we clean up the directory.
    {
        println!("Storage directory: {}\n", dir.display());

        let mut manager = PebbleManagerBuilder::new()
            .storage(FileStorage::new(&dir))
            .hot_capacity(4)
            .warm_capacity(4)
            .build::<Snapshot>()
            .unwrap();

        println!("Configuration: hot=4, warm=4\n");

        // Add 20 checkpoints. Each is 64 bytes of payload.
        for i in 1..=20 {
            let cp = Snapshot {
                id: i,
                payload: vec![i as u8; 64],
            };
            manager.add(cp).unwrap();

            let stats = manager.stats();
            println!(
                "add({i:>2})  hot={:<2} warm={:<2} buf={:<2} disk={:<2}  io={}",
                stats.red_pebble_count(),
                stats.warm_count(),
                stats.write_buffer_count(),
                stats.blue_pebble_count(),
                stats.io_operations(),
            );
        }

        println!("\n--- Flush ---\n");
        manager.flush().unwrap();

        let stats = manager.stats();
        println!(
            "After flush:  hot={}  warm={}  buf={}  disk={}  io={}",
            stats.red_pebble_count(),
            stats.warm_count(),
            stats.write_buffer_count(),
            stats.blue_pebble_count(),
            stats.io_operations(),
        );
        let file_count = fs::read_dir(&dir).unwrap().count();
        println!("Files on disk: {}", file_count);

        println!("\n--- Load from disk ---\n");

        // Load checkpoint 1 back from disk into hot tier.
        let io_before = manager.stats().io_operations();
        let cp = manager.load(1).unwrap().clone();
        let io_after = manager.stats().io_operations();
        println!(
            "Loaded checkpoint {}: {} bytes payload, {} I/O",
            cp.id,
            cp.payload.len(),
            io_after - io_before,
        );
        println!("Checkpoint 1 in hot tier: {}", manager.is_hot(1));

        // Manager drops here — flushes any remaining items to disk automatically.
    }

    fs::remove_dir_all(&dir).ok();
    println!("\nCleaned up {}", dir.display());
}
