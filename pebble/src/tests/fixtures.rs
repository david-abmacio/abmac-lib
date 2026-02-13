//! Shared test fixtures for PebbleManager tests.

use crate::manager::{Checkpointable, DirectStorage};
use crate::storage::InMemoryStorage;

/// Checkpoint type used across manager, branch, and safety tests.
#[derive(Debug, Clone, bytecast::DeriveToBytes, bytecast::DeriveFromBytes)]
pub struct TestCheckpoint {
    pub id: u64,
    pub data: alloc::string::String,
}

impl Checkpointable for TestCheckpoint {
    type Id = u64;
    type RebuildError = ();

    fn checkpoint_id(&self) -> Self::Id {
        self.id
    }

    fn compute_from_dependencies(
        base: Self,
        _deps: &hashbrown::HashMap<Self::Id, &Self>,
    ) -> core::result::Result<Self, Self::RebuildError> {
        Ok(base)
    }
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
