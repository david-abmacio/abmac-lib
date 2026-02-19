//! Basic PebbleManager operations: add, insert, eviction, load, compress, stats.

use super::*;

#[test]
fn test_basic_add_and_get() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        10,
        false,
    );

    let cp = TestCheckpoint {
        id: 1,
        data: "test".into(),
    };

    manager.add(cp, &[]).unwrap();
    assert!(manager.is_hot(1));
    assert_eq!(manager.get(1).unwrap().data, "test");
}

#[test]
fn test_insert_zero_copy() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        10,
        false,
    );

    // Use insert with constructor closure
    let id = manager
        .insert(&[], || TestCheckpoint {
            id: 42,
            data: "constructed".into(),
        })
        .unwrap();

    assert_eq!(id, 42);
    assert!(manager.is_hot(42));
    assert_eq!(manager.get(42).unwrap().data, "constructed");
}

#[test]
fn test_insert_with_dependencies() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        10,
        false,
    );

    // Add parent checkpoint
    manager
        .add(
            TestCheckpoint {
                id: 1,
                data: "parent".into(),
            },
            &[],
        )
        .unwrap();

    // Insert child with dependency using closure
    let child_id = manager
        .insert(&[1], || TestCheckpoint {
            id: 2,
            data: "child".into(),
        })
        .unwrap();

    assert_eq!(child_id, 2);
    assert!(manager.is_hot(1));
    assert!(manager.is_hot(2));
}

#[test]
fn test_eviction() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        2,
        false,
    );

    // Fill up fast memory
    for i in 0..5 {
        let cp = TestCheckpoint {
            id: i,
            data: alloc::format!("data{i}"),
        };
        manager.add(cp, &[]).unwrap();
    }

    // Some should have been evicted
    assert!(manager.red_count() <= 2);
    // Flush so evicted items reach storage
    manager.flush().unwrap();
    assert!(manager.blue_count() > 0);
    // All 5 checkpoints are accounted for across tiers
    assert_eq!(manager.red_count() + manager.blue_count(), 5);
}

#[test]
fn test_load_from_storage() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        2,
        false,
    );

    // Add checkpoints
    for i in 0..4 {
        let cp = TestCheckpoint {
            id: i,
            data: alloc::format!("data{i}"),
        };
        manager.add(cp, &[]).unwrap();
    }

    // Flush so evicted items reach cold storage
    manager.flush().unwrap();

    // Find one in storage and load it
    let mut loaded_id = None;
    for i in 0..4 {
        if manager.is_in_storage(i) {
            loaded_id = Some(i);
            break;
        }
    }

    let id = loaded_id.expect("at least one checkpoint should be in storage");
    let cp = manager.load(id).unwrap();
    assert_eq!(cp.id, id);
    assert!(manager.is_hot(id));
}

#[test]
fn test_compress() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        100,
        false,
    );

    // Add many checkpoints
    for i in 0..100 {
        let cp = TestCheckpoint {
            id: i,
            data: alloc::format!("data{i}"),
        };
        manager.add(cp, &[]).unwrap();
    }

    // All in fast memory
    assert_eq!(manager.red_count(), 100);
    assert_eq!(manager.blue_count(), 0);

    // Force compression
    let compressed = manager.compress().unwrap();
    assert!(compressed > 0);
    assert!(
        manager.red_count() < 100,
        "compress should reduce red pebbles"
    );
    manager.flush().unwrap();
    assert!(manager.blue_count() > 0);
    // All 100 checkpoints are accounted for across tiers
    assert_eq!(manager.red_count() + manager.blue_count(), 100);
}

#[test]
fn test_stats() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        10,
        false,
    );

    for i in 0..5 {
        let cp = TestCheckpoint {
            id: i,
            data: alloc::format!("data{i}"),
        };
        manager.add(cp, &[]).unwrap();
    }

    let stats = manager.stats();
    assert_eq!(stats.checkpoints_added(), 5);
    assert_eq!(stats.red_pebble_count(), 5);
    assert_eq!(stats.blue_pebble_count(), 0);
    assert_eq!(stats.io_operations(), 0, "no evictions means no I/O");
}
