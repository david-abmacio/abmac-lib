//! Builder tests for PebbleManager.

use super::*;
use crate::manager::PebbleBuilder;

#[test]
fn build_clamps_zero_hot_capacity() {
    // hot_capacity=0 is silently clamped to 1 (infallible build)
    let mut manager = PebbleBuilder::new()
        .cold(test_cold())
        .warm(NoWarm)
        .log(DropSpout)
        .hot_capacity(0)
        .build::<TestCheckpoint>();

    // Should work with clamped capacity of 1
    manager
        .add(
            TestCheckpoint {
                id: 1,
                data: "ok".into(),
            },
            &[],
        )
        .unwrap();
    assert!(manager.is_hot(1));
}

#[test]
fn hint_total_checkpoints_computes_sqrt() {
    // sqrt(10_000) = 100, so hot_capacity should be 100
    // Verify via eviction threshold: add 100 items without eviction
    let mut manager = PebbleBuilder::new()
        .cold(test_cold())
        .warm(NoWarm)
        .log(DropSpout)
        .hint_total_checkpoints(10_000)
        .build::<TestCheckpoint>();

    for i in 0..100 {
        manager
            .add(
                TestCheckpoint {
                    id: i,
                    data: alloc::format!("data{i}"),
                },
                &[],
            )
            .unwrap();
    }
    // 100 items, hot_capacity=100: all in hot, none evicted
    assert_eq!(manager.stats().red_pebble_count(), 100);
    assert_eq!(manager.stats().blue_pebble_count(), 0);

    // 101st triggers eviction
    manager
        .add(
            TestCheckpoint {
                id: 100,
                data: "overflow".into(),
            },
            &[],
        )
        .unwrap();
    assert!(manager.stats().blue_pebble_count() > 0 || manager.stats().red_pebble_count() <= 100);
}

#[test]
fn builder_warm_capacity_configurable() {
    use crate::manager::{RingCold, WarmCache};

    let cold = RingCold::<u64, _, 64>::new(InMemoryStorage::<u64, u128, 8>::new());
    let mut manager = PebbleBuilder::new()
        .cold(cold)
        .warm(WarmCache::<TestCheckpoint>::with_capacity(2))
        .log(DropSpout)
        .hot_capacity(4)
        .build::<TestCheckpoint>();

    // hot=4, warm_capacity=2
    // Adds 0..6: 4 hot, 2 warm (full)
    for i in 0..6 {
        manager
            .add(
                TestCheckpoint {
                    id: i,
                    data: alloc::format!("data{i}"),
                },
                &[],
            )
            .unwrap();
    }

    let stats = manager.stats();
    assert_eq!(stats.red_pebble_count(), 4);
    assert_eq!(stats.warm_count(), 2);

    // 7th add overflows warm -> write buffer
    manager
        .add(
            TestCheckpoint {
                id: 6,
                data: "overflow".into(),
            },
            &[],
        )
        .unwrap();

    let stats = manager.stats();
    assert_eq!(stats.warm_count(), 2);
    assert!(stats.write_buffer_count() > 0 || stats.blue_pebble_count() > 0);
}
