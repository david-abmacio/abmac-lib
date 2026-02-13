//! Tests for PebbleManager.

use alloc::vec;
use alloc::vec::Vec;

use crate::manager::{
    CheckpointSerializer, Checkpointable, DirectStorage, NoWarm, PebbleManager, PebbleManagerError,
};
use crate::storage::InMemoryStorage;
use crate::strategy::Strategy;

// Test fixtures

#[derive(Debug, Clone)]
struct TestCheckpoint {
    id: u64,
    data: alloc::string::String,
    deps: Vec<u64>,
}

impl Checkpointable for TestCheckpoint {
    type Id = u64;
    type RebuildError = ();

    fn checkpoint_id(&self) -> Self::Id {
        self.id
    }

    fn dependencies(&self) -> &[Self::Id] {
        &self.deps
    }

    fn compute_from_dependencies(
        base: Self,
        _deps: &hashbrown::HashMap<Self::Id, &Self>,
    ) -> core::result::Result<Self, Self::RebuildError> {
        Ok(base)
    }
}

/// Test serialization error type with meaningful context.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TestSerializerError {
    TooShort { expected: usize, actual: usize },
}

impl core::fmt::Display for TestSerializerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShort { expected, actual } => {
                write!(
                    f,
                    "buffer too short: expected {expected} bytes, got {actual}"
                )
            }
        }
    }
}

struct TestSerializer;

impl CheckpointSerializer<TestCheckpoint> for TestSerializer {
    type Error = TestSerializerError;

    fn serialize(&self, value: &TestCheckpoint) -> core::result::Result<Vec<u8>, Self::Error> {
        // Format: id (8) + dep_count (4) + deps (8 each) + data
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&value.id.to_le_bytes());
        bytes.extend_from_slice(&(value.deps.len() as u32).to_le_bytes());
        for dep in &value.deps {
            bytes.extend_from_slice(&dep.to_le_bytes());
        }
        bytes.extend_from_slice(value.data.as_bytes());
        Ok(bytes)
    }

    fn deserialize(&self, bytes: &[u8]) -> core::result::Result<TestCheckpoint, Self::Error> {
        // Need at least id (8) + dep_count (4) = 12 bytes
        if bytes.len() < 12 {
            return Err(TestSerializerError::TooShort {
                expected: 12,
                actual: bytes.len(),
            });
        }
        let id = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
        let dep_count = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;

        let deps_end = 12 + dep_count * 8;
        if bytes.len() < deps_end {
            return Err(TestSerializerError::TooShort {
                expected: deps_end,
                actual: bytes.len(),
            });
        }

        let mut deps = Vec::with_capacity(dep_count);
        for i in 0..dep_count {
            let start = 12 + i * 8;
            let dep = u64::from_le_bytes(bytes[start..start + 8].try_into().unwrap());
            deps.push(dep);
        }

        let data = alloc::string::String::from_utf8_lossy(&bytes[deps_end..]).into_owned();
        Ok(TestCheckpoint { id, data, deps })
    }
}

/// Helper: create a DirectStorage cold tier for tests.
fn test_cold() -> DirectStorage<InMemoryStorage<u64, u128, 8>, TestSerializer> {
    DirectStorage::new(InMemoryStorage::<u64, u128, 8>::new(), TestSerializer)
}

// Basic operations

#[test]
fn test_basic_add_and_get() {
    let mut manager = PebbleManager::new(test_cold(), NoWarm, Strategy::default(), 10);

    let cp = TestCheckpoint {
        id: 1,
        data: "test".into(),
        deps: vec![],
    };

    manager.add(cp).unwrap();
    assert!(manager.is_hot(1));
    assert_eq!(manager.get(1).unwrap().data, "test");
}

#[test]
fn test_insert_zero_copy() {
    let mut manager = PebbleManager::new(test_cold(), NoWarm, Strategy::default(), 10);

    // Use insert with constructor closure
    let id = manager
        .insert(|| TestCheckpoint {
            id: 42,
            data: "constructed".into(),
            deps: vec![],
        })
        .unwrap();

    assert_eq!(id, 42);
    assert!(manager.is_hot(42));
    assert_eq!(manager.get(42).unwrap().data, "constructed");
}

#[test]
fn test_insert_with_dependencies() {
    let mut manager = PebbleManager::new(test_cold(), NoWarm, Strategy::default(), 10);

    // Add parent checkpoint
    manager
        .add(TestCheckpoint {
            id: 1,
            data: "parent".into(),
            deps: vec![],
        })
        .unwrap();

    // Insert child with dependency using closure
    let child_id = manager
        .insert(|| TestCheckpoint {
            id: 2,
            data: "child".into(),
            deps: vec![1],
        })
        .unwrap();

    assert_eq!(child_id, 2);
    assert!(manager.is_hot(1));
    assert!(manager.is_hot(2));
}

#[test]
fn test_eviction() {
    let mut manager = PebbleManager::new(test_cold(), NoWarm, Strategy::default(), 2);

    // Fill up fast memory
    for i in 0..5 {
        let cp = TestCheckpoint {
            id: i,
            data: alloc::format!("data{i}"),
            deps: vec![],
        };
        manager.add(cp).unwrap();
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
    let mut manager = PebbleManager::new(test_cold(), NoWarm, Strategy::default(), 2);

    // Add checkpoints
    for i in 0..4 {
        let cp = TestCheckpoint {
            id: i,
            data: alloc::format!("data{i}"),
            deps: vec![],
        };
        manager.add(cp).unwrap();
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
    let mut manager = PebbleManager::new(test_cold(), NoWarm, Strategy::default(), 100);

    // Add many checkpoints
    for i in 0..100 {
        let cp = TestCheckpoint {
            id: i,
            data: alloc::format!("data{i}"),
            deps: vec![],
        };
        manager.add(cp).unwrap();
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
    let mut manager = PebbleManager::new(test_cold(), NoWarm, Strategy::default(), 10);

    for i in 0..5 {
        let cp = TestCheckpoint {
            id: i,
            data: alloc::format!("data{i}"),
            deps: vec![],
        };
        manager.add(cp).unwrap();
    }

    let stats = manager.stats();
    assert_eq!(stats.checkpoints_added(), 5);
    assert_eq!(stats.red_pebble_count(), 5);
    assert_eq!(stats.blue_pebble_count(), 0);
    assert_eq!(stats.io_operations(), 0, "no evictions means no I/O");
}

// Rebuild tests

#[test]
fn test_rebuild_simple() {
    let mut manager = PebbleManager::new(test_cold(), NoWarm, Strategy::default(), 2);

    // Add a chain of checkpoints: 0 -> 1 -> 2 -> 3
    for i in 0..4 {
        let cp = TestCheckpoint {
            id: i,
            data: alloc::format!("data{i}"),
            deps: if i > 0 { vec![i - 1] } else { vec![] },
        };
        manager.add(cp).unwrap();
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
    let mut manager = PebbleManager::new(test_cold(), NoWarm, Strategy::default(), 10);

    let cp = TestCheckpoint {
        id: 1,
        data: "test".into(),
        deps: vec![],
    };
    manager.add(cp).unwrap();

    // Rebuild from fast memory should work
    let rebuilt = manager.rebuild(1).unwrap();
    assert_eq!(rebuilt.id, 1);
    assert_eq!(rebuilt.data, "test");
}

#[test]
fn test_rebuild_not_found() {
    let mut manager: PebbleManager<TestCheckpoint, _, _> =
        PebbleManager::new(test_cold(), NoWarm, Strategy::default(), 10);

    // Rebuild non-existent checkpoint should fail
    let result = manager.rebuild(999);
    assert!(matches!(result, Err(PebbleManagerError::NeverAdded { .. })));
}

// Theoretical validation tests

#[test]
fn test_theoretical_validation_space_bound() {
    // 100 nodes with hot_capacity=10 (sqrt(100)=10, 2x=20)
    // Should satisfy space bound since 10 <= 20
    let mut manager = PebbleManager::new(test_cold(), NoWarm, Strategy::default(), 10);

    for i in 0..100 {
        let cp = TestCheckpoint {
            id: i,
            data: alloc::format!("data{i}"),
            deps: vec![],
        };
        manager.add(cp).unwrap();
    }

    let validation = manager.validate_theoretical_bounds();
    assert!(
        validation.space_bound_satisfied(),
        "Space bound should be satisfied: hot=10, sqrt(100)=10, 2x=20"
    );
    assert_eq!(validation.expected_max_space(), 10);
    assert_eq!(validation.total_nodes(), 100);

    // Space complexity ratio: hot_capacity=10, sqrt(100)=10 → ratio=1.0
    let stats = manager.stats();
    assert!(
        (stats.space_complexity_ratio() - 1.0).abs() < 0.01,
        "Space ratio should be ~1.0, got {}",
        stats.space_complexity_ratio()
    );
}

#[test]
fn test_theoretical_validation_space_bound_exceeded() {
    // 16 nodes with hot_capacity=100
    // sqrt(16) = 4, 2x = 8, but we have 100
    // Should NOT satisfy space bound
    let mut manager = PebbleManager::new(test_cold(), NoWarm, Strategy::default(), 100);

    for i in 0..16 {
        let cp = TestCheckpoint {
            id: i,
            data: alloc::format!("data{i}"),
            deps: vec![],
        };
        manager.add(cp).unwrap();
    }

    let validation = manager.validate_theoretical_bounds();
    assert!(
        !validation.space_bound_satisfied(),
        "Space bound should NOT be satisfied: hot=100 > 2*sqrt(16)=8"
    );
    assert_eq!(validation.expected_max_space(), 4);
}

#[test]
fn test_theoretical_validation_io_bound() {
    // Verify that real I/O is tracked and the io_optimality_ratio reflects it.
    // With hot=10 and 20 nodes, eviction writes bump the I/O counter.
    // The DAG bound (3.0x) is an asymptotic guarantee — small workloads
    // may exceed it — so we test that the ratio is computed correctly
    // rather than that it satisfies the bound at toy scale.
    let mut manager = PebbleManager::new(test_cold(), NoWarm, Strategy::default(), 10);

    for i in 0..20 {
        let cp = TestCheckpoint {
            id: i,
            data: alloc::format!("data{i}"),
            deps: vec![],
        };
        manager.add(cp).unwrap();
    }

    manager.flush().unwrap();

    // Load one item from cold — this is a real read I/O
    let cold_id = (0..20)
        .find(|&i| manager.is_in_storage(i))
        .expect("at least one checkpoint should be in storage");
    manager.load(cold_id).unwrap();

    let stats = manager.stats();
    assert!(stats.io_operations() > 0, "should have performed real I/O");
    assert!(stats.theoretical_min_io() > 0, "min I/O should be nonzero");
    assert!(
        stats.io_optimality_ratio() > 1.0,
        "ratio should exceed 1.0 with eviction overhead (got {})",
        stats.io_optimality_ratio(),
    );

    // Validation should report the bound check consistently
    let validation = manager.validate_theoretical_bounds();
    let expected = stats.io_optimality_ratio() <= 3.0;
    assert_eq!(
        validation.io_bound_satisfied(),
        expected,
        "io_bound_satisfied should match manual ratio check",
    );
}

// Recovery tests

use crate::storage::RecoveryMode;

#[test]
fn test_recover_cold_start() {
    // Fresh storage with no checkpoints
    let cold = test_cold();
    let (manager, result) =
        PebbleManager::<TestCheckpoint, _, _>::recover(cold, NoWarm, Strategy::default(), 10)
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
    // First, populate storage with checkpoints
    let mut manager = PebbleManager::new(test_cold(), NoWarm, Strategy::default(), 2);

    // Add checkpoints - with small hot_capacity, some will go to storage
    for i in 0..5 {
        let cp = TestCheckpoint {
            id: i,
            data: alloc::format!("data{i}"),
            deps: vec![],
        };
        manager.add(cp).unwrap();
    }

    // Force eviction to storage
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

    // Recover from storage
    let cold = DirectStorage::new(storage, TestSerializer);
    let (recovered_manager, result) =
        PebbleManager::<TestCheckpoint, _, _>::recover(cold, NoWarm, Strategy::default(), 10)
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
    // Create storage with dependent checkpoints
    let mut manager = PebbleManager::new(test_cold(), NoWarm, Strategy::default(), 2);

    // Add a chain: 0 -> 1 -> 2
    for i in 0..3 {
        let cp = TestCheckpoint {
            id: i,
            data: alloc::format!("chain{i}"),
            deps: if i > 0 { vec![i - 1] } else { vec![] },
        };
        manager.add(cp).unwrap();
    }

    // Force to storage - compress until no more can be evicted
    loop {
        let evicted = manager.compress().unwrap();
        if evicted == 0 {
            break;
        }
    }
    manager.flush().unwrap();

    // Record how many are in storage before recovery
    let in_storage = manager.blue_count();
    assert!(in_storage > 0, "Should have some checkpoints in storage");

    // Get storage for recovery
    let storage = core::mem::replace(
        manager.cold_mut().storage_mut(),
        InMemoryStorage::<u64, u128, 8>::new(),
    );

    // Recover - should recover what was in storage
    let cold = DirectStorage::new(storage, TestSerializer);
    let (recovered_manager, result) =
        PebbleManager::<TestCheckpoint, _, _>::recover(cold, NoWarm, Strategy::default(), 10)
            .unwrap();

    assert_eq!(result.mode, RecoveryMode::WarmRestart);
    assert!(result.integrity_errors.is_empty());
    // Recovery only sees what was persisted to storage
    assert_eq!(recovered_manager.len(), in_storage);
}

// Cold-buffer tier integration

#[cfg(feature = "cold-buffer")]
mod cold_buffer {
    use super::*;
    use crate::manager::{RingCold, WarmCache};
    use crate::storage::RecoverableStorage;

    type BufferedMgr = PebbleManager<
        TestCheckpoint,
        RingCold<u64, InMemoryStorage<u64, u128, 8>, TestSerializer, 64>,
        WarmCache<TestCheckpoint>,
    >;

    /// Helper: create a RingCold + WarmCache manager for cold-buffer tests.
    fn test_spill_manager(hot_capacity: usize) -> BufferedMgr {
        let cold = RingCold::new(InMemoryStorage::<u64, u128, 8>::new(), TestSerializer);
        let warm = WarmCache::new();
        PebbleManager::new(cold, warm, Strategy::default(), hot_capacity)
    }

    #[test]
    fn eviction_lands_in_warm_tier() {
        let mut manager = test_spill_manager(2);

        for i in 0..5 {
            manager
                .add(TestCheckpoint {
                    id: i,
                    data: alloc::format!("data{i}"),
                    deps: vec![],
                })
                .unwrap();
        }

        // Evicted items land in warm tier, not cold storage
        assert!(manager.is_in_warm(0) || manager.is_in_warm(1));
        assert_eq!(manager.blue_count(), 0, "nothing flushed to cold yet");
    }

    #[test]
    fn load_promotes_from_warm_tier() {
        let mut manager = test_spill_manager(2);

        for i in 0..4 {
            manager
                .add(TestCheckpoint {
                    id: i,
                    data: alloc::format!("data{i}"),
                    deps: vec![],
                })
                .unwrap();
        }

        // Find one in the warm tier
        let mut warm_id = None;
        for i in 0..4 {
            if manager.is_in_warm(i) {
                warm_id = Some(i);
                break;
            }
        }
        let id = warm_id.expect("should have item in warm cache");

        // Load promotes from warm -> hot (no I/O)
        let io_before = manager.stats().io_operations();
        let cp = manager.load(id).unwrap();
        assert_eq!(cp.id, id);
        assert!(manager.is_hot(id));
        assert!(!manager.is_in_warm(id));
        // No I/O since it came from warm tier
        assert_eq!(manager.stats().io_operations(), io_before);
    }

    #[test]
    fn flush_drains_warm_to_storage() {
        let mut manager = test_spill_manager(2);

        for i in 0..5 {
            manager
                .add(TestCheckpoint {
                    id: i,
                    data: alloc::format!("data{i}"),
                    deps: vec![],
                })
                .unwrap();
        }

        let warm = manager.stats().warm_count();
        assert!(warm > 0);

        manager.flush().unwrap();

        assert_eq!(manager.stats().warm_count(), 0);
        assert!(manager.blue_count() > 0);
    }

    #[test]
    fn rebuild_finds_deps_in_warm_tier() {
        let mut manager = test_spill_manager(2);

        // Chain: 0 -> 1 -> 2 -> 3
        for i in 0..4 {
            manager
                .add(TestCheckpoint {
                    id: i,
                    data: alloc::format!("data{i}"),
                    deps: if i > 0 { vec![i - 1] } else { vec![] },
                })
                .unwrap();
        }

        // Flush to cold so we can test full rebuild
        manager.flush().unwrap();

        // Find one in storage
        for i in 0..4 {
            if manager.is_in_storage(i) {
                let rebuilt = manager.rebuild(i).unwrap();
                assert_eq!(rebuilt.id, i);
                return;
            }
        }
        panic!("expected at least one checkpoint in storage");
    }

    #[test]
    fn stats_includes_warm_count() {
        let mut manager = test_spill_manager(2);

        for i in 0..4 {
            manager
                .add(TestCheckpoint {
                    id: i,
                    data: alloc::format!("data{i}"),
                    deps: vec![],
                })
                .unwrap();
        }

        let stats = manager.stats();
        assert!(stats.warm_count() > 0);
        assert_eq!(
            stats.red_pebble_count() + stats.blue_pebble_count() + stats.warm_count(),
            4
        );
    }

    #[test]
    fn remove_from_warm_tier() {
        let mut manager = test_spill_manager(2);

        for i in 0..4 {
            manager
                .add(TestCheckpoint {
                    id: i,
                    data: alloc::format!("data{i}"),
                    deps: vec![],
                })
                .unwrap();
        }

        // Find one in warm cache and remove it
        for i in 0..4 {
            if manager.is_in_warm(i) {
                assert!(manager.remove(i));
                assert!(!manager.contains(i));
                return;
            }
        }
        panic!("expected item in warm cache");
    }

    #[test]
    fn contains_checks_all_tiers() {
        let mut manager = test_spill_manager(2);

        for i in 0..4 {
            manager
                .add(TestCheckpoint {
                    id: i,
                    data: alloc::format!("data{i}"),
                    deps: vec![],
                })
                .unwrap();
        }

        // All 4 should be contained somewhere
        for i in 0..4 {
            assert!(manager.contains(i), "checkpoint {i} should exist");
        }
        assert!(!manager.contains(99));
    }

    #[test]
    fn write_buffer_batches_before_storage() {
        let mut manager = test_spill_manager(2);

        // Add enough to fill warm cache and push into write buffer.
        // hot=2, so each add beyond 2 evicts to warm.
        // Warm holds 64, so after 66 adds: 2 in hot, 64 in warm (exactly full).
        // We need 67+ adds to trigger warm overflow into write buffer.
        for i in 0..70 {
            manager
                .add(TestCheckpoint {
                    id: i,
                    data: alloc::format!("data{i}"),
                    deps: vec![],
                })
                .unwrap();
        }

        // 70 adds, hot=2: 2 hot, 64 warm (full), 4 overflow -> write buffer
        let stats = manager.stats();
        assert_eq!(stats.red_pebble_count(), 2);
        assert_eq!(stats.warm_count(), 64);
        // The 4 overflowed items are serialized into the write buffer
        assert_eq!(stats.write_buffer_count(), 4, "4 items should be buffered");
        assert_eq!(stats.blue_pebble_count(), 4, "4 items serialized so far");

        // Storage should not yet have received all items (write buffer batches)
        let storage_count_before_flush = manager.cold().storage().iter_metadata().count();

        // Flush drains everything
        manager.flush().unwrap();

        let stats_after = manager.stats();
        assert_eq!(stats_after.warm_count(), 0, "warm drained");
        assert_eq!(stats_after.write_buffer_count(), 0, "write buffer drained");

        // Storage now has everything that was serialized
        let storage_count_after_flush = manager.cold().storage().iter_metadata().count();
        assert!(
            storage_count_after_flush >= storage_count_before_flush,
            "flush should have moved items to storage"
        );
    }

    #[test]
    fn flush_drains_write_buffer_to_storage() {
        let mut manager = test_spill_manager(2);

        // Fill warm, then flush to push through write buffer to storage
        for i in 0..10 {
            manager
                .add(TestCheckpoint {
                    id: i,
                    data: alloc::format!("data{i}"),
                    deps: vec![],
                })
                .unwrap();
        }

        // Before flush: some items in warm, some possibly in write buffer
        let before = manager.stats();
        assert!(before.warm_count() > 0 || before.write_buffer_count() > 0);

        manager.flush().unwrap();

        let after = manager.stats();
        assert_eq!(after.warm_count(), 0);
        assert_eq!(after.write_buffer_count(), 0);
        // All serialized items reached storage
        assert!(after.blue_pebble_count() > 0);
    }

    #[test]
    fn load_from_storage_through_write_buffer() {
        let mut manager = test_spill_manager(2);

        for i in 0..6 {
            manager
                .add(TestCheckpoint {
                    id: i,
                    data: alloc::format!("data{i}"),
                    deps: vec![],
                })
                .unwrap();
        }

        // Flush everything to storage
        manager.flush().unwrap();

        // Find one in storage and load it back
        let mut loaded = false;
        for i in 0..6 {
            if manager.is_in_storage(i) {
                let cp = manager.load(i).unwrap();
                assert_eq!(cp.id, i);
                assert!(manager.is_hot(i));
                loaded = true;
                break;
            }
        }
        assert!(loaded, "should have loaded a checkpoint from storage");
    }
}

// Builder

#[test]
fn build_rejects_zero_hot_capacity() {
    use crate::manager::PebbleManagerBuilder;

    let result = PebbleManagerBuilder::new()
        .cold(test_cold())
        .warm(NoWarm)
        .hot_capacity(0)
        .build::<TestCheckpoint>();

    assert!(result.is_err());
    assert_eq!(result.err().unwrap(), crate::BuilderError::ZeroHotCapacity,);
}

#[test]
fn hint_total_checkpoints_computes_sqrt() {
    use crate::manager::PebbleManagerBuilder;

    // sqrt(10_000) = 100, so hot_capacity should be 100
    // Verify via eviction threshold: add 100 items without eviction
    let mut manager = PebbleManagerBuilder::new()
        .cold(test_cold())
        .warm(NoWarm)
        .hint_total_checkpoints(10_000)
        .build::<TestCheckpoint>()
        .unwrap();

    for i in 0..100 {
        manager
            .add(TestCheckpoint {
                id: i,
                data: alloc::format!("data{i}"),
                deps: vec![],
            })
            .unwrap();
    }
    // 100 items, hot_capacity=100: all in hot, none evicted
    assert_eq!(manager.stats().red_pebble_count(), 100);
    assert_eq!(manager.stats().blue_pebble_count(), 0);

    // 101st triggers eviction
    manager
        .add(TestCheckpoint {
            id: 100,
            data: "overflow".into(),
            deps: vec![],
        })
        .unwrap();
    assert!(manager.stats().blue_pebble_count() > 0 || manager.stats().red_pebble_count() <= 100);
}

#[cfg(feature = "cold-buffer")]
#[test]
fn builder_warm_capacity_configurable() {
    use crate::manager::{PebbleManagerBuilder, RingCold, WarmCache};

    let cold = RingCold::<u64, _, TestSerializer, 64>::new(
        InMemoryStorage::<u64, u128, 8>::new(),
        TestSerializer,
    );
    let mut manager = PebbleManagerBuilder::new()
        .cold(cold)
        .warm(WarmCache::<TestCheckpoint>::with_capacity(2))
        .hot_capacity(4)
        .build::<TestCheckpoint>()
        .unwrap();

    // hot=4, warm_capacity=2
    // Adds 0..6: 4 hot, 2 warm (full)
    for i in 0..6 {
        manager
            .add(TestCheckpoint {
                id: i,
                data: alloc::format!("data{i}"),
                deps: vec![],
            })
            .unwrap();
    }

    let stats = manager.stats();
    assert_eq!(stats.red_pebble_count(), 4);
    assert_eq!(stats.warm_count(), 2);

    // 7th add overflows warm -> write buffer
    manager
        .add(TestCheckpoint {
            id: 6,
            data: "overflow".into(),
            deps: vec![],
        })
        .unwrap();

    let stats = manager.stats();
    assert_eq!(stats.warm_count(), 2);
    assert!(stats.write_buffer_count() > 0 || stats.blue_pebble_count() > 0);
}
