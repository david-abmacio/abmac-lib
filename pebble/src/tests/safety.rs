//! Tests for compile-time safety types (CheckpointRef, CapacityGuard).

use spout::DropSpout;

use crate::manager::{DirectStorage, Manifest, NoWarm, PebbleManager, PebbleManagerError};
use crate::storage::InMemoryStorage;
use crate::strategy::Strategy;

// Reuse the test fixtures from manager tests.

#[derive(Debug, Clone)]
struct Cp {
    id: u64,
}

impl crate::manager::Checkpointable for Cp {
    type Id = u64;
    type RebuildError = ();

    fn checkpoint_id(&self) -> u64 {
        self.id
    }

    fn compute_from_dependencies(
        base: Self,
        _deps: &hashbrown::HashMap<Self::Id, &Self>,
    ) -> core::result::Result<Self, ()> {
        Ok(base)
    }
}

struct Ser;

impl crate::manager::CheckpointSerializer<Cp> for Ser {
    type Error = &'static str;

    fn serialize(&self, cp: &Cp) -> core::result::Result<alloc::vec::Vec<u8>, &'static str> {
        Ok(cp.id.to_le_bytes().to_vec())
    }

    fn deserialize(&self, bytes: &[u8]) -> core::result::Result<Cp, &'static str> {
        if bytes.len() < 8 {
            return Err("too short");
        }
        let id = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
        Ok(Cp { id })
    }
}

fn cold() -> DirectStorage<InMemoryStorage<u64, u128, 8>, Ser> {
    DirectStorage::new(InMemoryStorage::<u64, u128, 8>::new(), Ser)
}

fn cp(id: u64) -> Cp {
    Cp { id }
}

// --- CheckpointRef tests ---

#[test]
fn test_add_ref_returns_token() {
    let mut mgr = PebbleManager::new(
        cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        4,
    );
    let token = mgr.add_ref(cp(1), &[]).unwrap();
    assert_eq!(token.id(), 1);
    assert!(mgr.is_hot(1));
}

#[test]
fn test_insert_ref_returns_token() {
    let mut mgr = PebbleManager::new(
        cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        4,
    );
    let token = mgr.insert_ref(&[], || cp(42)).unwrap();
    assert_eq!(token.id(), 42);
    assert!(mgr.is_hot(42));
}

#[test]
fn test_locate_found() {
    let mut mgr = PebbleManager::new(
        cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        4,
    );
    mgr.add(cp(10), &[]).unwrap();
    let token = mgr.locate(10);
    assert!(token.is_some());
    assert_eq!(token.unwrap().id(), 10);
}

#[test]
fn test_locate_not_found() {
    let mgr = PebbleManager::new(
        cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        4,
    );
    assert!(mgr.locate(999).is_none());
}

#[test]
fn test_load_ref() {
    let mut mgr = PebbleManager::new(
        cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        4,
    );
    let token = mgr.add_ref(cp(5), &[]).unwrap();
    let loaded = mgr.load_ref(token).unwrap();
    assert_eq!(loaded.id, 5);
}

#[test]
fn test_rebuild_ref() {
    let mut mgr = PebbleManager::new(
        cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        4,
    );
    let token = mgr.add_ref(cp(7), &[]).unwrap();
    let rebuilt = mgr.rebuild_ref(token).unwrap();
    assert_eq!(rebuilt.id, 7);
}

#[test]
fn test_stale_token() {
    let mut mgr = PebbleManager::new(
        cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        4,
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
    let mut mgr = PebbleManager::new(
        cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        4,
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
    let mut mgr = PebbleManager::new(
        cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        2,
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
    let mut mgr = PebbleManager::new(
        cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        4,
    );
    let guard = mgr.ensure_capacity().unwrap();
    let token = guard.insert(&[], || cp(77)).unwrap();
    assert_eq!(token.id(), 77);
    assert!(mgr.is_hot(77));
}
