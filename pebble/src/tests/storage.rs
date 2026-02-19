//! Tests for storage backends.

use alloc::vec;
use spout::Spout;

use crate::storage::{
    CheckpointLoader, CheckpointMetadata, InMemoryStorage, RecoverableStorage, StorageError,
};

// CheckpointMetadata tests

#[test]
fn checkpoint_metadata_dependencies() {
    // CId = u64, SId = u128, MAX_DEPS = 8
    let meta =
        CheckpointMetadata::<u64, u128, 8>::with_dependencies(100, &[1, 2, 3], 12345, 0).unwrap();
    assert_eq!(meta.dependencies(), &[1, 2, 3]);
    assert_eq!(meta.dep_count, 3);
}

// InMemoryStorage tests

#[test]
fn send_and_load() {
    // CId = u64, SId = u128, MAX_DEPS = 8
    let mut storage = InMemoryStorage::<u64, u128, 8>::new();
    let data = b"checkpoint data".to_vec();

    // Send via Spout
    storage.send((1, data.clone(), vec![]));

    // Load via CheckpointLoader
    let loaded = storage.load(1).unwrap();
    assert_eq!(loaded, data);
}

#[test]
fn load_not_found() {
    let storage = InMemoryStorage::<u64, u128, 8>::new();
    let result = storage.load(999);
    assert!(matches!(result, Err(StorageError::NotFound)));
}

#[test]
fn contains_check() {
    let mut storage = InMemoryStorage::<u64, u128, 8>::new();

    assert!(!storage.contains(1));
    storage.send((1, vec![1, 2, 3], vec![]));
    assert!(storage.contains(1));
}

#[test]
fn remove_checkpoint() {
    let mut storage = InMemoryStorage::<u64, u128, 8>::new();

    storage.send((1, vec![1, 2, 3], vec![]));
    assert_eq!(storage.len(), 1);

    let removed = storage.remove(1);
    assert!(removed);
    assert_eq!(storage.len(), 0);
    assert!(!storage.contains(1));
}

#[test]
fn metadata_iteration() {
    let mut storage = InMemoryStorage::<u64, u128, 8>::new();

    let meta1 = CheckpointMetadata::<u64, u128, 8>::with_dependencies(1, &[0], 100, 0).unwrap();
    let meta2 = CheckpointMetadata::<u64, u128, 8>::with_dependencies(2, &[1], 200, 0).unwrap();

    storage.store_with_metadata(1, vec![1, 2, 3], meta1);
    storage.store_with_metadata(2, vec![4, 5, 6], meta2);

    let collected: alloc::vec::Vec<_> = storage.iter_metadata().collect();
    assert_eq!(collected.len(), 2);
}

#[test]
fn get_metadata() {
    let mut storage = InMemoryStorage::<u64, u128, 8>::new();

    let meta = CheckpointMetadata::<u64, u128, 8>::with_dependencies(42, &[1, 2], 100, 0).unwrap();
    storage.store_with_metadata(42, vec![1, 2, 3], meta.clone());

    let retrieved = storage.get_metadata(42).unwrap();
    assert_eq!(retrieved.state_id, 42);
    assert_eq!(retrieved.dependencies(), &[1, 2]);
}

#[test]
fn configurable_max_deps() {
    // Test with smaller MAX_DEPS for embedded systems
    let meta4 =
        CheckpointMetadata::<u64, u128, 4>::with_dependencies(1, &[10, 20, 30], 0, 0).unwrap();
    assert_eq!(meta4.dependencies(), &[10, 20, 30]);
    assert_eq!(CheckpointMetadata::<u64, u128, 4>::max_dependencies(), 4);

    // Test with larger MAX_DEPS for complex DAGs
    let meta16 = CheckpointMetadata::<u64, u128, 16>::with_dependencies(
        2,
        &[1, 2, 3, 4, 5, 6, 7, 8, 9],
        0,
        0,
    )
    .unwrap();
    assert_eq!(meta16.dependencies().len(), 9);
    assert_eq!(CheckpointMetadata::<u64, u128, 16>::max_dependencies(), 16);

    // Default is 8
    assert_eq!(CheckpointMetadata::<u64, u128, 8>::max_dependencies(), 8);
}

#[test]
fn max_deps_exceeded_returns_error() {
    // Trying to store 5 deps in a CheckpointMetadata with MAX_DEPS=4 should return an error
    let result = CheckpointMetadata::<u64, u128, 4>::with_dependencies(1, &[1, 2, 3, 4, 5], 0, 0);
    assert!(matches!(
        result,
        Err(StorageError::TooManyDependencies { max: 4, count: 5 })
    ));
}

#[test]
fn session_id_no_tracking() {
    // Use () for no session tracking - zero overhead on embedded systems
    // CId = u64, SId = (), MAX_DEPS = 4
    let meta = CheckpointMetadata::<u64, (), 4>::new(1, 12345, ());
    assert_eq!(meta.session_id, ());
    assert_eq!(meta.session_timestamp(), None);

    // Storage with no session tracking
    let mut storage = InMemoryStorage::<u64, (), 4>::new();
    storage.send((1, vec![1, 2, 3], vec![]));
    assert_eq!(storage.len(), 1);
}

#[test]
fn session_id_with_timestamp() {
    use crate::storage::SessionId;

    // Custom ID type that encodes a timestamp
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
    struct TimestampedId(u64);

    impl SessionId for TimestampedId {
        fn timestamp(&self) -> Option<u64> {
            Some(self.0)
        }
    }

    // CId = u64, SId = TimestampedId, MAX_DEPS = 8
    let meta = CheckpointMetadata::<u64, TimestampedId, 8>::new(1, 0, TimestampedId(1234567890));
    assert_eq!(meta.session_timestamp(), Some(1234567890));
}

#[test]
fn generic_checkpoint_id_u128() {
    // Test using u128 as checkpoint ID
    let mut storage = InMemoryStorage::<u128, u128, 8>::new();

    let large_id: u128 = 0xDEAD_BEEF_CAFE_BABE_1234_5678_9ABC_DEF0;
    storage.send((large_id, vec![1, 2, 3], vec![]));

    assert!(storage.contains(large_id));
    let loaded = storage.load(large_id).unwrap();
    assert_eq!(loaded, vec![1, 2, 3]);
}
