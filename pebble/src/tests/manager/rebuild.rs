//! Rebuild tests for PebbleManager.

use super::*;

#[test]
fn test_rebuild_simple() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        2,
        false,
    );

    // Add a chain of checkpoints: 0 -> 1 -> 2 -> 3
    for i in 0..4 {
        let cp = TestCheckpoint {
            id: i,
            data: alloc::format!("data{i}"),
        };
        let deps = if i > 0 { vec![i - 1] } else { vec![] };
        manager.add(cp, &deps).unwrap();
    }

    // With hot_capacity=2, some should be evicted
    manager.flush().unwrap();
    assert!(
        manager.blue_count() > 0,
        "Some checkpoints should be in storage"
    );

    // Find a checkpoint in storage and rebuild it
    let storage_id = (0..4)
        .find(|&i| manager.is_in_storage(i))
        .expect("at least one checkpoint should be in storage");
    let rebuilt = manager.rebuild(storage_id).unwrap();
    assert_eq!(rebuilt.id, storage_id);
}

#[test]
fn test_rebuild_from_hot() {
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

    // Rebuild from fast memory should work
    let rebuilt = manager.rebuild(1).unwrap();
    assert_eq!(rebuilt.id, 1);
    assert_eq!(rebuilt.data, "test");
}

#[test]
fn test_rebuild_not_found() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        10,
        false,
    );

    // Rebuild non-existent checkpoint should fail
    let result = manager.rebuild(999);
    assert!(matches!(result, Err(PebbleManagerError::NeverAdded { .. })));
}
