//! Dirty-flag flush tests: hot-tier persistence to cold storage.

use super::*;
use crate::storage::RecoverableStorage;

#[test]
fn flush_persists_hot_items() {
    // Large hot_capacity so nothing gets evicted — all items stay hot.
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        100,
        false,
    );

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

    // All 5 in hot, none evicted.
    assert_eq!(manager.red_count(), 5);
    assert_eq!(manager.blue_count(), 0);

    // Before flush: nothing in cold storage.
    assert_eq!(manager.cold().storage().iter_metadata().count(), 0);

    // Flush persists dirty hot items to cold.
    manager.flush().unwrap();

    // Items still in hot (not evicted).
    assert_eq!(manager.red_count(), 5);
    // Items now also in cold storage.
    assert_eq!(manager.cold().storage().iter_metadata().count(), 5);
}

#[test]
fn flush_skips_clean_items() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        100,
        false,
    );

    // Add 3 items and flush.
    for i in 0..3 {
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
    let io_after_first_flush = manager.stats().io_operations();

    // Add 2 more items and flush again.
    for i in 3..5 {
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
    let io_after_second_flush = manager.stats().io_operations();

    // Second flush should only write the 2 new items, not re-write the 3 clean ones.
    assert_eq!(io_after_second_flush - io_after_first_flush, 2);
    assert_eq!(manager.cold().storage().iter_metadata().count(), 5);
}

#[test]
fn dirty_cleared_on_eviction() {
    // hot_capacity=2, so adding 3 items triggers eviction.
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        2,
        false,
    );

    for i in 0..3 {
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

    // Some items were evicted to cold. Record I/O from eviction.
    let io_after_adds = manager.stats().io_operations();

    // Flush should only persist the remaining dirty hot items,
    // not re-store already-evicted ones.
    manager.flush().unwrap();
    let io_after_flush = manager.stats().io_operations();

    // The flush should write only the dirty hot items (those not yet in cold).
    // Total items in cold should be 3 (all accounted for).
    assert_eq!(manager.cold().storage().iter_metadata().count(), 3);

    // Flush I/O should be less than 3 (some were already written by eviction).
    let flush_io = io_after_flush - io_after_adds;
    assert!(
        flush_io <= manager.red_count() as u64,
        "flush should only write dirty hot items, not re-store evicted ones"
    );
}

#[test]
fn load_from_cold_not_dirty() {
    // hot_capacity=2: adding 3 items evicts 1 to cold via blue_pebbles.
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        2,
        false,
    );

    for i in 0..3 {
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

    // Find the item that was evicted to cold (in blue_pebbles).
    let cold_id = (0..3)
        .find(|&i| manager.is_in_storage(i))
        .expect("should have an item in blue_pebbles");

    // Remove a hot item to make room so load won't trigger eviction.
    let hot_id = (0..3)
        .find(|&i| manager.is_hot(i) && i != cold_id)
        .expect("should have a hot item to remove");
    manager.remove(hot_id);
    assert_eq!(manager.red_count(), 1);

    let io_before_load = manager.stats().io_operations();

    // Load the cold item back into hot.
    manager.load(cold_id).unwrap();
    assert!(manager.is_hot(cold_id));

    // Flush again — the loaded-from-cold item should NOT be dirty.
    manager.flush().unwrap();
    let io_after = manager.stats().io_operations();

    // Only the load read costs I/O. Flush should add 0 (loaded item is clean).
    let io_delta = io_after - io_before_load;
    assert_eq!(
        io_delta, 1,
        "only the load should cost I/O, not the re-flush (got {io_delta})"
    );
}

#[test]
fn remove_clears_dirty() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        100,
        false,
    );

    manager
        .add(
            TestCheckpoint {
                id: 1,
                data: "ephemeral".into(),
            },
            &[],
        )
        .unwrap();

    // Remove before flush — should clear dirty flag.
    assert!(manager.remove(1));

    // Flush should write nothing.
    manager.flush().unwrap();
    assert_eq!(manager.cold().storage().iter_metadata().count(), 0);
    assert_eq!(manager.stats().io_operations(), 0);
}
