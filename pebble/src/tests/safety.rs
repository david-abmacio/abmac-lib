//! Tests for compile-time safety types (CheckpointRef, CapacityGuard).

use spout::DropSpout;

use crate::manager::{Manifest, NoWarm, PebbleManager, PebbleManagerError};
use crate::strategy::Strategy;

use super::fixtures::{TestCheckpoint, cp, test_cold};

// --- CheckpointRef tests ---

#[test]
fn test_add_ref_returns_token() {
    let mut mgr = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        4,
        false,
    );
    let token = mgr.add_ref(cp(1), &[]).unwrap();
    assert_eq!(token.id(), 1);
    assert!(mgr.is_hot(1));
}

#[test]
fn test_insert_ref_returns_token() {
    let mut mgr = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        4,
        false,
    );
    let token = mgr.insert_ref(&[], || cp(42)).unwrap();
    assert_eq!(token.id(), 42);
    assert!(mgr.is_hot(42));
}

#[test]
fn test_locate_found() {
    let mut mgr = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        4,
        false,
    );
    mgr.add(cp(10), &[]).unwrap();
    let token = mgr.locate(10);
    assert!(token.is_some());
    assert_eq!(token.unwrap().id(), 10);
}

#[test]
fn test_locate_not_found() {
    let mgr = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        4,
        false,
    );
    assert!(mgr.locate(999).is_none());
}

#[test]
fn test_load_ref() {
    let mut mgr = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        4,
        false,
    );
    let token = mgr.add_ref(cp(5), &[]).unwrap();
    let loaded = mgr.load_ref(token).unwrap();
    assert_eq!(loaded.id, 5);
}

#[test]
fn test_rebuild_ref() {
    let mut mgr = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        4,
        false,
    );
    let token = mgr.add_ref(cp(7), &[]).unwrap();
    let rebuilt = mgr.rebuild_ref(token).unwrap();
    assert_eq!(rebuilt.id, 7);
}

#[test]
fn test_stale_token() {
    let mut mgr = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        4,
        false,
    );
    let token = mgr.add_ref(cp(3), &[]).unwrap();
    mgr.remove(3);
    let result = mgr.load_ref(token);
    assert!(matches!(
        result,
        Err(PebbleManagerError::NeverAdded { state_id: 3 })
    ));
}

// --- CapacityGuard tests ---

#[test]
fn test_ensure_capacity_when_not_full() {
    let mut mgr = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        4,
        false,
    );
    mgr.add(cp(1), &[]).unwrap();
    // Hot tier has space — no eviction needed
    let guard = mgr.ensure_capacity().unwrap();
    let token = guard.store(cp(2), &[]).unwrap();
    assert_eq!(token.id(), 2);
    assert!(mgr.is_hot(1));
    assert!(mgr.is_hot(2));
}

#[test]
fn test_ensure_capacity_evicts_when_full() {
    let mut mgr = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        2,
        false,
    );
    mgr.add(cp(1), &[]).unwrap();
    mgr.add(cp(2), &[]).unwrap();
    assert_eq!(mgr.red_count(), 2);

    // Hot tier is full — ensure_capacity evicts, guard.add() succeeds
    let token = mgr.ensure_capacity().unwrap().store(cp(3), &[]).unwrap();
    assert_eq!(token.id(), 3);
    assert!(mgr.is_hot(3));
    // At least one of the originals was evicted
    assert!(mgr.red_count() <= 2);
}

#[test]
fn test_guard_insert() {
    let mut mgr = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        4,
        false,
    );
    let guard = mgr.ensure_capacity().unwrap();
    let token = guard.insert(&[], || cp(77)).unwrap();
    assert_eq!(token.id(), 77);
    assert!(mgr.is_hot(77));
}
