//! Tombstone tests for PebbleManager manifest entries.

use super::*;
use crate::manager::cold::ColdTier;
use crate::storage::CheckpointLoader;
use spout::CollectSpout;

#[test]
fn test_tombstone_recorded_on_cold_remove() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(CollectSpout::new()),
        Strategy::default(),
        2,
        false,
    );

    // Add 3 checkpoints — hot_capacity=2 so the first will be evicted to cold.
    for i in 0..3 {
        manager
            .add(
                TestCheckpoint {
                    id: i,
                    data: alloc::format!("d{i}"),
                },
                &[],
            )
            .unwrap();
    }

    // Find which checkpoint ended up in cold.
    let cold_id = (0..3).find(|&i| manager.is_in_storage(i)).unwrap();
    let seq_before = manager.manifest().seq();

    // Remove the cold checkpoint — should produce a tombstone.
    assert!(manager.remove(cold_id));

    let seq_after = manager.manifest().seq();
    assert_eq!(seq_after, seq_before + 1, "tombstone should increment seq");

    // Flush so manifest entries drain to the CollectSpout.
    manager.flush().unwrap();

    let entries = manager.manifest().spout().items();
    let tombstones: alloc::vec::Vec<_> = entries.iter().filter(|e| e.tombstone).collect();
    assert_eq!(tombstones.len(), 1);
    assert_eq!(tombstones[0].checkpoint_id, cold_id);

    // Cold data remains until compact() purges it.
    assert!(
        manager.cold().storage().contains(cold_id),
        "cold data should survive remove() until compact()"
    );

    manager.compact();
    assert!(
        !manager.cold().storage().contains(cold_id),
        "cold data should be purged after compact()"
    );
}

#[test]
fn test_no_tombstone_for_hot_remove() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(CollectSpout::new()),
        Strategy::default(),
        10,
        false,
    );

    manager
        .add(
            TestCheckpoint {
                id: 1,
                data: "hot".into(),
            },
            &[],
        )
        .unwrap();

    assert!(manager.is_hot(1));
    let seq_before = manager.manifest().seq();

    // Remove a hot checkpoint — no tombstone needed.
    assert!(manager.remove(1));

    assert_eq!(
        manager.manifest().seq(),
        seq_before,
        "hot remove should not write tombstone"
    );
}

#[test]
fn test_tombstone_sequencing() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(CollectSpout::new()),
        Strategy::default(),
        2,
        false,
    );

    // Add 3 to force eviction.
    for i in 0..3 {
        manager
            .add(
                TestCheckpoint {
                    id: i,
                    data: alloc::format!("d{i}"),
                },
                &[],
            )
            .unwrap();
    }

    let cold_id = (0..3).find(|&i| manager.is_in_storage(i)).unwrap();

    // Remove the cold checkpoint.
    assert!(manager.remove(cold_id));

    // Flush and inspect ordering.
    manager.flush().unwrap();

    let entries = manager.manifest().spout().items();
    let eviction = entries
        .iter()
        .find(|e| !e.tombstone && e.checkpoint_id == cold_id);
    let tombstone = entries
        .iter()
        .find(|e| e.tombstone && e.checkpoint_id == cold_id);

    assert!(eviction.is_some(), "should have eviction entry");
    assert!(tombstone.is_some(), "should have tombstone entry");
    assert!(
        tombstone.unwrap().seq > eviction.unwrap().seq,
        "tombstone seq ({}) should be greater than eviction seq ({})",
        tombstone.unwrap().seq,
        eviction.unwrap().seq,
    );
}

#[test]
fn test_cold_storage_cleaned_on_remove() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(CollectSpout::new()),
        Strategy::default(),
        2,
        false,
    );

    // Add 4 checkpoints — hot_capacity=2 so at least 2 will be evicted to cold.
    for i in 0..4 {
        manager
            .add(
                TestCheckpoint {
                    id: i,
                    data: alloc::format!("d{i}"),
                },
                &[],
            )
            .unwrap();
    }

    let cold_ids: alloc::vec::Vec<_> = (0..4).filter(|&i| manager.is_in_storage(i)).collect();
    assert!(!cold_ids.is_empty(), "should have items in cold storage");

    // Verify cold storage contains the data.
    for &id in &cold_ids {
        assert!(manager.cold().storage().contains(id));
    }

    // Remove cold checkpoints — data stays until compact().
    for &id in &cold_ids {
        assert!(manager.remove(id));
        assert!(
            manager.cold().storage().contains(id),
            "cold data for {id} should survive remove()"
        );
    }

    // compact() purges the data.
    manager.compact();
    for &id in &cold_ids {
        assert!(
            !manager.cold().storage().contains(id),
            "cold data for {id} should be gone after compact()"
        );
    }

    // Remaining items should still be accessible.
    let hot_ids: alloc::vec::Vec<_> = (0..4).filter(|i| !cold_ids.contains(i)).collect();
    for &id in &hot_ids {
        assert!(manager.is_hot(id));
    }
}

// --- Failing cold tier for WAL rollback tests ---

use crate::manager::cold::DirectStorageError;
use core::cell::Cell;

/// Cold tier that fails after `fail_after` successful stores.
struct FailingCold {
    pub inner: super::super::fixtures::TestCold,
    remaining: Cell<usize>,
}

impl FailingCold {
    fn new(fail_after: usize) -> Self {
        Self {
            inner: test_cold(),
            remaining: Cell::new(fail_after),
        }
    }
}

impl ColdTier<TestCheckpoint> for FailingCold {
    type Error = DirectStorageError;

    fn store(
        &mut self,
        id: u64,
        checkpoint: &TestCheckpoint,
        deps: &[u64],
    ) -> Result<(), Self::Error> {
        let r = self.remaining.get();
        if r == 0 {
            return Err(DirectStorageError::Storage {
                source: crate::storage::StorageError::NotFound,
            });
        }
        self.remaining.set(r - 1);
        self.inner.store(id, checkpoint, deps)
    }

    fn load(&self, id: u64) -> Result<TestCheckpoint, Self::Error> {
        self.inner.load(id)
    }

    fn contains(&self, id: u64) -> bool {
        ColdTier::<TestCheckpoint>::contains(&self.inner, id)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        ColdTier::<TestCheckpoint>::flush(&mut self.inner)
    }

    fn remove(&mut self, id: u64) -> Result<bool, Self::Error> {
        ColdTier::<TestCheckpoint>::remove(&mut self.inner, id)
    }

    fn buffered_count(&self) -> usize {
        0
    }
}

#[test]
fn test_compensating_tombstone_on_cold_store_failure() {
    // Allow 1 successful store, then fail all subsequent stores.
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        FailingCold::new(1),
        NoWarm,
        Manifest::new(CollectSpout::new()),
        Strategy::default(),
        2, // hot_capacity=2
        false,
    );

    // Add 3 checkpoints. hot_capacity=2, so the first eviction triggers
    // persist_to_cold. The first store succeeds (fail_after=1).
    manager
        .add(
            TestCheckpoint {
                id: 0,
                data: "a".into(),
            },
            &[],
        )
        .unwrap();
    manager
        .add(
            TestCheckpoint {
                id: 1,
                data: "b".into(),
            },
            &[],
        )
        .unwrap();

    // This add triggers eviction. The evicted checkpoint's cold store
    // succeeds (first store). But now remaining=0.
    manager
        .add(
            TestCheckpoint {
                id: 2,
                data: "c".into(),
            },
            &[],
        )
        .unwrap();

    // After 3 adds with hot_capacity=2, one eviction succeeded.
    // Manifest has 1 entry (eviction record). seq=1.
    let seq_after_first_eviction = manager.manifest().seq();
    assert_eq!(
        seq_after_first_eviction, 1,
        "one successful eviction record"
    );

    // Add a 4th checkpoint to trigger another eviction. This time
    // cold.store() fails. persist_to_cold should write the WAL entry
    // (seq=1) then the compensating tombstone (seq=2), then return Err.
    let result = manager.add(
        TestCheckpoint {
            id: 3,
            data: "d".into(),
        },
        &[],
    );
    assert!(result.is_err(), "second eviction should fail");

    // Manifest should now have 3 entries:
    //   seq=0: eviction record for the first (successful) cold store
    //   seq=1: eviction record for the failed cold store
    //   seq=2: compensating tombstone for the failed cold store
    let seq_after_failure = manager.manifest().seq();
    assert_eq!(
        seq_after_failure, 3,
        "should have eviction + failed eviction + compensating tombstone = 3 entries"
    );

    // The checkpoint that failed to store should NOT be in cold storage.
    // Only the first evicted checkpoint should be there.
    let evicted_id = (0..3).find(|&i| manager.is_in_storage(i)).unwrap();
    let cold_count = (0..4)
        .filter(|&i| manager.cold().inner.storage().contains(i))
        .count();
    assert_eq!(cold_count, 1, "only the first eviction should be in cold");
    assert!(
        manager.cold().inner.storage().contains(evicted_id),
        "first evicted checkpoint should be in cold"
    );
}
