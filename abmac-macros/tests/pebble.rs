#![cfg(feature = "pebble")]

use pebble::Checkpointable;

#[derive(Clone, pebble::Checkpoint)]
struct TestState {
    #[checkpoint(id)]
    id: u64,
    value: u32,
}

#[test]
fn checkpoint_id_returns_annotated_field() {
    let state = TestState { id: 42, value: 100 };
    assert_eq!(state.checkpoint_id(), 42);
}

#[test]
fn compute_from_dependencies_returns_base() {
    let state = TestState { id: 1, value: 99 };
    let deps = hashbrown::HashMap::new();
    let result = TestState::compute_from_dependencies(state.clone(), &deps);
    let rebuilt = result.unwrap();
    assert_eq!(rebuilt.id, 1);
    assert_eq!(rebuilt.value, 99);
}

#[test]
fn bytecast_roundtrip() {
    use bytecast::{FromBytes, ToBytesExt};

    let state = TestState { id: 7, value: 256 };
    let bytes = state.to_vec().unwrap();
    let (decoded, _) = TestState::from_bytes(&bytes).unwrap();
    assert_eq!(decoded.id, 7);
    assert_eq!(decoded.value, 256);
}
