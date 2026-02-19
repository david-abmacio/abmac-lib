//! Tests for compact() — physical purge and orphan cascade.

use super::super::fixtures::cp;
use super::*;
use crate::storage::CheckpointLoader;
use spout::CollectSpout;

/// remove() flags but does not delete cold data. compact() purges it.
#[test]
fn test_remove_flags_compact_purges() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(CollectSpout::new()),
        Strategy::default(),
        2,
        false,
    );

    manager.add(cp(1), &[]).unwrap();
    manager.add(cp(2), &[]).unwrap();
    manager.add(cp(3), &[]).unwrap();

    let cold_id = (1..=3).find(|&i| manager.is_in_storage(i)).unwrap();

    // Data still in cold storage after remove.
    manager.remove(cold_id);
    assert!(
        manager.cold().storage().contains(cold_id),
        "cold data should survive remove()"
    );

    // compact() physically deletes it.
    let purged = manager.compact();
    assert!(purged >= 1);
    assert!(
        !manager.cold().storage().contains(cold_id),
        "cold data should be gone after compact()"
    );
}

/// A→B: remove A, compact purges orphaned B.
#[test]
fn test_compact_cascades_to_orphaned_child() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(CollectSpout::new()),
        Strategy::default(),
        10,
        false,
    );

    manager.add(cp(1), &[]).unwrap();
    manager.add(cp(2), &[1]).unwrap();

    manager.remove(1);

    // B is still in the DAG (orphaned, not yet purged).
    assert!(manager.contains(2), "orphan should survive remove()");

    // compact() purges the orphan.
    manager.compact();
    assert!(!manager.contains(2), "orphan should be purged by compact()");
}

/// A→C, B→C: remove A, compact does NOT purge C (B keeps it alive).
#[test]
fn test_compact_no_cascade_shared_parent() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(CollectSpout::new()),
        Strategy::default(),
        10,
        false,
    );

    manager.add(cp(1), &[]).unwrap();
    manager.add(cp(2), &[]).unwrap();
    manager.add(cp(3), &[1, 2]).unwrap();

    manager.remove(1);
    manager.compact();

    assert!(
        manager.contains(3),
        "child with remaining parent should survive"
    );
    assert!(manager.contains(2));
}

/// A→B→D, A→C→D: remove A, compact purges B, C, and D.
#[test]
fn test_compact_cascade_diamond() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(CollectSpout::new()),
        Strategy::default(),
        10,
        false,
    );

    manager.add(cp(1), &[]).unwrap();
    manager.add(cp(2), &[1]).unwrap();
    manager.add(cp(3), &[1]).unwrap();
    manager.add(cp(4), &[2, 3]).unwrap();

    manager.remove(1);
    manager.compact();

    assert!(!manager.contains(2));
    assert!(!manager.contains(3));
    assert!(!manager.contains(4));
}

/// A→B→C→D: remove A, compact purges entire chain.
#[test]
fn test_compact_cascade_chain() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(CollectSpout::new()),
        Strategy::default(),
        10,
        false,
    );

    manager.add(cp(1), &[]).unwrap();
    manager.add(cp(2), &[1]).unwrap();
    manager.add(cp(3), &[2]).unwrap();
    manager.add(cp(4), &[3]).unwrap();

    manager.remove(1);
    manager.compact();

    assert!(!manager.contains(2));
    assert!(!manager.contains(3));
    assert!(!manager.contains(4));
}

/// Removing a leaf — compact has nothing to cascade.
#[test]
fn test_compact_leaf_no_cascade() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(CollectSpout::new()),
        Strategy::default(),
        10,
        false,
    );

    manager.add(cp(1), &[]).unwrap();
    manager.add(cp(2), &[1]).unwrap();

    manager.remove(2);
    manager.compact();

    assert!(manager.contains(1), "parent should survive");
}

/// Purged cold-tier orphans get tombstones recorded.
#[test]
fn test_compact_cascade_tombstones() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(CollectSpout::new()),
        Strategy::default(),
        2,
        false,
    );

    // Build a chain. With hot_capacity=2, earlier nodes evict to cold.
    manager.add(cp(1), &[]).unwrap();
    manager.add(cp(2), &[1]).unwrap();
    manager.add(cp(3), &[2]).unwrap();

    manager.flush().unwrap();

    let cold_before: alloc::vec::Vec<_> = (1..=3).filter(|&i| manager.is_in_storage(i)).collect();

    // Remove root and compact.
    manager.remove(1);
    manager.compact();
    manager.flush().unwrap();

    let entries = manager.manifest().spout().items();
    let tombstones: alloc::vec::Vec<_> = entries
        .iter()
        .filter(|e| e.tombstone)
        .map(|e| e.checkpoint_id)
        .collect();

    // Every cold-tier node in the cascade should have a tombstone.
    for &id in &cold_before {
        assert!(
            tombstones.contains(&id),
            "cold node {id} should have a tombstone after compact"
        );
    }
}

/// compact() with no tombstoned items is a no-op.
#[test]
fn test_compact_noop() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(CollectSpout::new()),
        Strategy::default(),
        10,
        false,
    );

    manager.add(cp(1), &[]).unwrap();
    manager.add(cp(2), &[1]).unwrap();

    let purged = manager.compact();
    assert_eq!(purged, 0);
    assert!(manager.contains(1));
    assert!(manager.contains(2));
}

/// verify() skips tombstone entries — tombstoned IDs don't count as coverage.
#[test]
fn test_verify_skips_tombstones() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(CollectSpout::new()),
        Strategy::default(),
        2,
        false,
    );

    manager.add(cp(1), &[]).unwrap();
    manager.add(cp(2), &[]).unwrap();
    manager.add(cp(3), &[]).unwrap();

    let cold_id = (1..=3).find(|&i| manager.is_in_storage(i)).unwrap();
    manager.remove(cold_id);
    manager.compact();

    manager.flush().unwrap();

    let entries = manager.manifest().spout().items().to_vec();
    let result = manager.verify(entries);
    assert!(
        result.is_consistent,
        "verification should pass after compact; lost: {:?}",
        result.lost
    );
}
