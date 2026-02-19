//! ParallelCold integration tests.

use super::*;
use crate::manager::ParallelCold;
use crate::manager::cold::ColdTier;
use crate::storage::CheckpointLoader;
use spout::CollectSpout;

type TestStorage = InMemoryStorage<u64, u128, 8>;
type TestParallel = ParallelCold<u64, TestStorage, 8>;

fn test_parallel_cold(num_workers: usize) -> TestParallel {
    ParallelCold::new(InMemoryStorage::new(), num_workers)
}

/// Helper to disambiguate the `ColdTier<TestCheckpoint>` impl.
fn as_cold(cold: &mut TestParallel) -> &mut impl ColdTier<TestCheckpoint> {
    cold
}

#[test]
fn test_store_and_load() {
    let mut cold = test_parallel_cold(2);
    let cp = TestCheckpoint {
        id: 1,
        data: "hello".into(),
    };
    as_cold(&mut cold).store(1, &cp).unwrap();
    as_cold(&mut cold).flush().unwrap();

    let loaded: TestCheckpoint = as_cold(&mut cold).load(1).unwrap();
    assert_eq!(loaded.id, 1);
    assert_eq!(loaded.data, "hello");
}

#[test]
fn test_contains_after_flush() {
    let mut cold = test_parallel_cold(2);
    let cp = TestCheckpoint {
        id: 1,
        data: "data".into(),
    };
    as_cold(&mut cold).store(1, &cp).unwrap();

    // Pending items aren't visible to contains until flushed.
    assert!(!as_cold(&mut cold).contains(1));

    as_cold(&mut cold).flush().unwrap();
    assert!(as_cold(&mut cold).contains(1));
}

#[test]
fn test_buffered_count() {
    let mut cold = test_parallel_cold(4);

    assert_eq!(as_cold(&mut cold).buffered_count(), 0);

    // Store fewer than num_workers — stays pending.
    for i in 0..3 {
        as_cold(&mut cold)
            .store(
                i,
                &TestCheckpoint {
                    id: i,
                    data: alloc::format!("d{i}"),
                },
            )
            .unwrap();
    }
    assert_eq!(as_cold(&mut cold).buffered_count(), 3);

    as_cold(&mut cold).flush().unwrap();
    assert_eq!(as_cold(&mut cold).buffered_count(), 0);
}

#[test]
fn test_remove_from_storage() {
    let mut cold = test_parallel_cold(2);
    let cp = TestCheckpoint {
        id: 1,
        data: "removeme".into(),
    };
    as_cold(&mut cold).store(1, &cp).unwrap();
    as_cold(&mut cold).flush().unwrap();

    assert!(as_cold(&mut cold).contains(1));
    assert_eq!(as_cold(&mut cold).remove(1).unwrap(), true);
    assert!(!as_cold(&mut cold).contains(1));

    // Removing again returns false.
    assert_eq!(as_cold(&mut cold).remove(1).unwrap(), false);
}

#[test]
fn test_batch_distribution() {
    let num_workers = 4;
    let mut cold = test_parallel_cold(num_workers);

    // Store exactly num_workers items — triggers auto-batch.
    for i in 0..num_workers as u64 {
        as_cold(&mut cold)
            .store(
                i,
                &TestCheckpoint {
                    id: i,
                    data: alloc::format!("d{i}"),
                },
            )
            .unwrap();
    }

    // After storing num_workers items, pending should have been drained
    // by the auto-batch, but items may still be in worker rings.
    as_cold(&mut cold).flush().unwrap();

    // All items should be in storage.
    for i in 0..num_workers as u64 {
        assert!(cold.storage().contains(i), "missing checkpoint {i}");
    }
}

#[test]
fn test_manager_eviction_to_parallel() {
    let cold = test_parallel_cold(2);
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        cold,
        NoWarm,
        Manifest::new(spout::DropSpout),
        Strategy::default(),
        2,
        false,
    );

    // Add 4 items — hot_capacity=2 so at least 2 evict to cold.
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

    manager.flush().unwrap();

    let cold_ids: alloc::vec::Vec<_> = (0..4).filter(|&i| manager.is_in_storage(i)).collect();
    assert!(!cold_ids.is_empty(), "should have items in cold storage");

    // Load from cold back into hot.
    let cp = manager.load(cold_ids[0]).unwrap().clone();
    assert_eq!(cp.id, cold_ids[0]);
}

#[test]
fn test_multi_flush_cycles() {
    let mut cold = test_parallel_cold(2);

    // First cycle: store and flush.
    for i in 0..4 {
        as_cold(&mut cold)
            .store(
                i,
                &TestCheckpoint {
                    id: i,
                    data: alloc::format!("cycle1_{i}"),
                },
            )
            .unwrap();
    }
    as_cold(&mut cold).flush().unwrap();

    // All first-cycle items in storage.
    for i in 0..4 {
        assert!(
            cold.storage().contains(i),
            "missing checkpoint {i} after first flush"
        );
    }

    // Second cycle: store more items and flush again (pool reused, no respawn).
    for i in 10..14 {
        as_cold(&mut cold)
            .store(
                i,
                &TestCheckpoint {
                    id: i,
                    data: alloc::format!("cycle2_{i}"),
                },
            )
            .unwrap();
    }
    as_cold(&mut cold).flush().unwrap();

    // All items from both cycles present.
    for i in 0..4 {
        assert!(
            cold.storage().contains(i),
            "missing checkpoint {i} after second flush"
        );
    }
    for i in 10..14 {
        assert!(
            cold.storage().contains(i),
            "missing checkpoint {i} after second flush"
        );
    }

    // Third cycle: verify load still works.
    let loaded: TestCheckpoint = as_cold(&mut cold).load(2).unwrap();
    assert_eq!(loaded.id, 2);
    let loaded: TestCheckpoint = as_cold(&mut cold).load(12).unwrap();
    assert_eq!(loaded.id, 12);
}

#[test]
fn test_manager_remove_cold_parallel() {
    let cold = test_parallel_cold(2);
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        cold,
        NoWarm,
        Manifest::new(CollectSpout::new()),
        Strategy::default(),
        2,
        false,
    );

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

    let cold_id = (0..4).find(|&i| manager.is_in_storage(i)).unwrap();

    // Remove the cold checkpoint — tombstone + storage cleanup.
    assert!(manager.remove(cold_id));

    // Verify storage is cleaned.
    assert!(!manager.cold().storage().contains(cold_id));

    // Verify tombstone was recorded.
    manager.flush().unwrap();
    let entries = manager.manifest().spout().items();
    let tombstones: alloc::vec::Vec<_> = entries.iter().filter(|e| e.tombstone).collect();
    assert_eq!(tombstones.len(), 1);
    assert_eq!(tombstones[0].checkpoint_id, cold_id);
}
