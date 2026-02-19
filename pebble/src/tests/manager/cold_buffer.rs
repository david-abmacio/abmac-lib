//! Cold-buffer tier integration tests (RingCold + WarmCache).

use super::*;
use crate::manager::{RingCold, WarmCache};
use crate::storage::RecoverableStorage;

type BufferedMgr = PebbleManager<
    TestCheckpoint,
    RingCold<u64, InMemoryStorage<u64, u128, 8>, 64>,
    WarmCache<TestCheckpoint>,
    DropSpout,
>;

/// Helper: create a RingCold + WarmCache manager for cold-buffer tests.
fn test_spill_manager(hot_capacity: usize) -> BufferedMgr {
    let cold = RingCold::new(InMemoryStorage::<u64, u128, 8>::new());
    let warm = WarmCache::new();
    PebbleManager::<TestCheckpoint, _, _, _>::new(
        cold,
        warm,
        Manifest::new(DropSpout),
        Strategy::default(),
        hot_capacity,
        false,
    )
}

#[test]
fn eviction_lands_in_warm_tier() {
    let mut manager = test_spill_manager(2);

    for i in 0..5 {
        manager
            .add(
                TestCheckpoint {
                    id: i,
                    data: alloc::format!("data{i}"),
                },
                &[],
            )
            .unwrap();
    }

    assert!(manager.is_in_warm(0) || manager.is_in_warm(1));
    assert_eq!(manager.blue_count(), 0, "nothing flushed to cold yet");
}

#[test]
fn load_promotes_from_warm_tier() {
    let mut manager = test_spill_manager(2);

    for i in 0..4 {
        manager
            .add(
                TestCheckpoint {
                    id: i,
                    data: alloc::format!("data{i}"),
                },
                &[],
            )
            .unwrap();
    }

    let mut warm_id = None;
    for i in 0..4 {
        if manager.is_in_warm(i) {
            warm_id = Some(i);
            break;
        }
    }
    let id = warm_id.expect("should have item in warm cache");

    let io_before = manager.stats().io_operations();
    let cp = manager.load(id).unwrap();
    assert_eq!(cp.id, id);
    assert!(manager.is_hot(id));
    assert!(!manager.is_in_warm(id));
    assert_eq!(manager.stats().io_operations(), io_before);
}

#[test]
fn flush_drains_warm_to_storage() {
    let mut manager = test_spill_manager(2);

    for i in 0..5 {
        manager
            .add(
                TestCheckpoint {
                    id: i,
                    data: alloc::format!("data{i}"),
                },
                &[],
            )
            .unwrap();
    }

    let warm = manager.stats().warm_count();
    assert!(warm > 0);

    manager.flush().unwrap();

    assert_eq!(manager.stats().warm_count(), 0);
    assert!(manager.blue_count() > 0);
}

#[test]
fn rebuild_finds_deps_in_warm_tier() {
    let mut manager = test_spill_manager(2);

    for i in 0..4 {
        let deps = if i > 0 { vec![i - 1] } else { vec![] };
        manager
            .add(
                TestCheckpoint {
                    id: i,
                    data: alloc::format!("data{i}"),
                },
                &deps,
            )
            .unwrap();
    }

    manager.flush().unwrap();

    for i in 0..4 {
        if manager.is_in_storage(i) {
            let rebuilt = manager.rebuild(i).unwrap();
            assert_eq!(rebuilt.id, i);
            return;
        }
    }
    panic!("expected at least one checkpoint in storage");
}

#[test]
fn stats_includes_warm_count() {
    let mut manager = test_spill_manager(2);

    for i in 0..4 {
        manager
            .add(
                TestCheckpoint {
                    id: i,
                    data: alloc::format!("data{i}"),
                },
                &[],
            )
            .unwrap();
    }

    let stats = manager.stats();
    assert!(stats.warm_count() > 0);
    assert_eq!(
        stats.red_pebble_count() + stats.blue_pebble_count() + stats.warm_count(),
        4
    );
}

#[test]
fn remove_from_warm_tier() {
    let mut manager = test_spill_manager(2);

    for i in 0..4 {
        manager
            .add(
                TestCheckpoint {
                    id: i,
                    data: alloc::format!("data{i}"),
                },
                &[],
            )
            .unwrap();
    }

    for i in 0..4 {
        if manager.is_in_warm(i) {
            assert!(manager.remove(i));
            assert!(!manager.contains(i));
            return;
        }
    }
    panic!("expected item in warm cache");
}

#[test]
fn contains_checks_all_tiers() {
    let mut manager = test_spill_manager(2);

    for i in 0..4 {
        manager
            .add(
                TestCheckpoint {
                    id: i,
                    data: alloc::format!("data{i}"),
                },
                &[],
            )
            .unwrap();
    }

    for i in 0..4 {
        assert!(manager.contains(i), "checkpoint {i} should exist");
    }
    assert!(!manager.contains(99));
}

#[test]
fn write_buffer_batches_before_storage() {
    let mut manager = test_spill_manager(2);

    for i in 0..70 {
        manager
            .add(
                TestCheckpoint {
                    id: i,
                    data: alloc::format!("data{i}"),
                },
                &[],
            )
            .unwrap();
    }

    let stats = manager.stats();
    assert_eq!(stats.red_pebble_count(), 2);
    assert_eq!(stats.warm_count(), 64);
    assert_eq!(stats.write_buffer_count(), 4, "4 items should be buffered");
    assert_eq!(stats.blue_pebble_count(), 4, "4 items serialized so far");

    let storage_count_before_flush = manager.cold().storage().iter_metadata().count();

    manager.flush().unwrap();

    let stats_after = manager.stats();
    assert_eq!(stats_after.warm_count(), 0, "warm drained");
    assert_eq!(stats_after.write_buffer_count(), 0, "write buffer drained");

    let storage_count_after_flush = manager.cold().storage().iter_metadata().count();
    assert!(
        storage_count_after_flush >= storage_count_before_flush,
        "flush should have moved items to storage"
    );
}

#[test]
fn flush_drains_write_buffer_to_storage() {
    let mut manager = test_spill_manager(2);

    for i in 0..10 {
        manager
            .add(
                TestCheckpoint {
                    id: i,
                    data: alloc::format!("data{i}"),
                },
                &[],
            )
            .unwrap();
    }

    let before = manager.stats();
    assert!(before.warm_count() > 0 || before.write_buffer_count() > 0);

    manager.flush().unwrap();

    let after = manager.stats();
    assert_eq!(after.warm_count(), 0);
    assert_eq!(after.write_buffer_count(), 0);
    assert!(after.blue_pebble_count() > 0);
}

#[test]
fn warm_hit_and_cold_load_stats() {
    let mut manager = test_spill_manager(2);

    for i in 0..6 {
        manager
            .add(
                TestCheckpoint {
                    id: i,
                    data: alloc::format!("data{i}"),
                },
                &[],
            )
            .unwrap();
    }

    assert_eq!(manager.stats().warm_hits(), 0);
    assert_eq!(manager.stats().cold_loads(), 0);

    // Load from warm tier.
    let warm_id = (0..6).find(|&i| manager.is_in_warm(i)).unwrap();
    manager.load(warm_id).unwrap();
    assert_eq!(manager.stats().warm_hits(), 1);
    assert_eq!(manager.stats().cold_loads(), 0);

    // Flush everything to cold so we can load from there.
    manager.flush().unwrap();

    let cold_id = (0..6).find(|&i| manager.is_in_storage(i)).unwrap();
    manager.load(cold_id).unwrap();
    assert_eq!(manager.stats().cold_loads(), 1);

    // Hot hit should not increment either counter.
    let hot_id = (0..6).find(|&i| manager.is_hot(i)).unwrap();
    let warm_before = manager.stats().warm_hits();
    let cold_before = manager.stats().cold_loads();
    manager.load(hot_id).unwrap();
    assert_eq!(manager.stats().warm_hits(), warm_before);
    assert_eq!(manager.stats().cold_loads(), cold_before);
}

#[test]
fn load_from_storage_through_write_buffer() {
    let mut manager = test_spill_manager(2);

    for i in 0..6 {
        manager
            .add(
                TestCheckpoint {
                    id: i,
                    data: alloc::format!("data{i}"),
                },
                &[],
            )
            .unwrap();
    }

    manager.flush().unwrap();

    let mut loaded = false;
    for i in 0..6 {
        if manager.is_in_storage(i) {
            let cp = manager.load(i).unwrap();
            assert_eq!(cp.id, i);
            assert!(manager.is_hot(i));
            loaded = true;
            break;
        }
    }
    assert!(loaded, "should have loaded a checkpoint from storage");
}
