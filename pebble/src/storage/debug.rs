//! Debug-only file-backed storage for development testing.
//!
//! Available only in debug builds with `std` enabled.
//! Writes checkpoints to a temporary directory, providing persistence
//! across restarts without requiring a production storage backend.

extern crate std;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use alloc::format;
use alloc::vec::Vec;

use spout::Spout;

use super::{CheckpointLoader, StorageError};

/// File-backed storage for debug builds.
///
/// Stores each checkpoint as a separate file (`<dir>/<id>.bin`) in a
/// temporary directory. Useful for testing persistence during development
/// without wiring up a production storage backend.
///
/// Only available in debug builds with `std`.
/// Does not exist in release — forces choosing a real backend.
#[derive(Debug)]
pub struct DebugFileStorage {
    dir: PathBuf,
    written: HashMap<u64, ()>,
}

impl DebugFileStorage {
    /// Create debug storage in a temp directory.
    ///
    /// Creates a `pebble_debug/` subdirectory under the system temp dir.
    pub fn new() -> Self {
        let dir = std::env::temp_dir().join("pebble_debug");
        Self::with_dir(&dir)
    }

    /// Create debug storage in a specific directory.
    pub fn with_dir(dir: &Path) -> Self {
        fs::create_dir_all(dir).expect("create debug storage dir");

        // Discover existing files so `contains` works across restarts.
        let mut written = HashMap::new();
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                if let Some(stem) = entry.path().file_stem()
                    && let Some(id) = stem.to_str().and_then(|s| s.parse::<u64>().ok())
                {
                    written.insert(id, ());
                }
            }
        }

        Self {
            dir: dir.to_path_buf(),
            written,
        }
    }

    /// Path to the storage directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path_for(&self, id: u64) -> PathBuf {
        self.dir.join(format!("{id}.bin"))
    }
}

impl Default for DebugFileStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl Spout<(u64, Vec<u8>)> for DebugFileStorage {
    type Error = core::convert::Infallible;

    fn send(&mut self, (id, bytes): (u64, Vec<u8>)) -> Result<(), Self::Error> {
        let path = self.path_for(id);
        fs::write(&path, &bytes).expect("write debug checkpoint file");
        self.written.insert(id, ());
        Ok(())
    }
}

impl CheckpointLoader<u64> for DebugFileStorage {
    fn load(&self, state_id: u64) -> Result<Vec<u8>, StorageError> {
        let path = self.path_for(state_id);
        fs::read(&path).map_err(|_| StorageError::NotFound)
    }

    fn contains(&self, state_id: u64) -> bool {
        self.written.contains_key(&state_id)
    }
}
