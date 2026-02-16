//! Recovery tests for PebbleManager.

use super::*;
use crate::storage::{RecoverableStorage, RecoveryMode};

#[test]
fn test_recover_cold_start() {
    let cold = test_cold();
    let (manager, result) = PebbleManager::<TestCheckpoint, _, _, _>::recover(
        cold,
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        10,
        false,
    )
    .unwrap();

    assert_eq!(result.mode, RecoveryMode::ColdStart);
    assert_eq!(result.checkpoints_loaded, 0);
    assert_eq!(result.dag_nodes_rebuilt, 0);
    assert!(result.latest_state_id.is_none());
    assert!(result.integrity_errors.is_empty());
    assert!(manager.is_empty());
}

#[test]
fn test_recover_warm_restart() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        2,
        false,
    );

    for i in 0..5 {
        let cp = TestCheckpoint {
            id: i,
            data: alloc::format!("data{i}"),
        };
        manager.add(cp, &[]).unwrap();
    }

    manager.compress().unwrap();
    manager.flush().unwrap();
    assert!(
        manager.blue_count() > 0,
        "Should have checkpoints in storage"
    );

    // Get the storage out to simulate restart
    let storage = core::mem::replace(
        manager.cold_mut().storage_mut(),
        InMemoryStorage::<u64, u128, 8>::new(),
    );

    let cold = DirectStorage::new(storage);
    let (recovered_manager, result) = PebbleManager::<TestCheckpoint, _, _, _>::recover(
        cold,
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        10,
        false,
    )
    .unwrap();

    assert_eq!(result.mode, RecoveryMode::WarmRestart);
    assert!(result.checkpoints_loaded > 0);
    assert_eq!(result.dag_nodes_rebuilt, result.checkpoints_loaded);
    assert!(result.latest_state_id.is_some());
    assert!(result.integrity_errors.is_empty());
    assert!(!recovered_manager.is_empty());
}

#[test]
fn test_recover_with_dependencies() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        2,
        false,
    );

    // Add a chain: 0 -> 1 -> 2
    for i in 0..3 {
        let cp = TestCheckpoint {
            id: i,
            data: alloc::format!("chain{i}"),
        };
        let deps = if i > 0 { vec![i - 1] } else { vec![] };
        manager.add(cp, &deps).unwrap();
    }

    loop {
        let evicted = manager.compress().unwrap();
        if evicted == 0 {
            break;
        }
    }
    manager.flush().unwrap();

    // After flush, cold storage holds both evicted items (blue) and
    // dirty-flushed hot items. Use iter_metadata to get the true count.
    let in_cold = manager.cold().storage().iter_metadata().count();
    assert!(in_cold > 0, "Should have some checkpoints in storage");

    let storage = core::mem::replace(
        manager.cold_mut().storage_mut(),
        InMemoryStorage::<u64, u128, 8>::new(),
    );

    let cold = DirectStorage::new(storage);
    let (recovered_manager, result) = PebbleManager::<TestCheckpoint, _, _, _>::recover(
        cold,
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        10,
        false,
    )
    .unwrap();

    assert_eq!(result.mode, RecoveryMode::WarmRestart);
    assert!(result.integrity_errors.is_empty());
    assert_eq!(recovered_manager.len(), in_cold);
}
