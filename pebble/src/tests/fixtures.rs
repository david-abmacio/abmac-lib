//! Shared test fixtures for PebbleManager tests.

use crate::manager::DirectStorage;
use crate::storage::InMemoryStorage;

/// Checkpoint type used across manager, branch, and safety tests.
#[derive(Debug, Clone, crate::Checkpoint)]
pub struct TestCheckpoint {
    #[checkpoint(id)]
    pub id: u64,
    pub data: alloc::string::String,
}

/// Cold tier type used in tests.
pub type TestCold = DirectStorage<InMemoryStorage<u64, u128, 8>>;

/// Create a fresh cold tier for tests.
pub fn test_cold() -> TestCold {
    DirectStorage::new(InMemoryStorage::<u64, u128, 8>::new())
}

/// Shorthand: create a `TestCheckpoint` with the given ID.
pub fn cp(id: u64) -> TestCheckpoint {
    TestCheckpoint {
        id,
        data: alloc::format!("cp-{id}"),
    }
}
