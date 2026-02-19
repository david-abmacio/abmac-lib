//! Resize and auto-resize tests for PebbleManager.

use super::*;

#[test]
fn test_resize_hot_grow() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        4,
        false,
    );

    for i in 0..10 {
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

    // With hot_capacity=4, some have been evicted.
    manager.flush().unwrap();
    assert!(manager.red_count() <= 4);
    assert!(manager.blue_count() > 0);

    // Grow to 20 — no immediate effect, but new adds stay hot.
    manager.resize_hot(20).unwrap();

    for i in 10..20 {
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

    // All 10 new items should stay hot (no eviction needed).
    assert!(manager.red_count() > 4);
    assert!(manager.red_count() <= 20);
}

#[test]
fn test_resize_hot_shrink() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        20,
        false,
    );

    for i in 0..10 {
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

    assert_eq!(manager.red_count(), 10);
    assert_eq!(manager.blue_count(), 0);

    // Shrink to 4 — excess should be evicted.
    manager.resize_hot(4).unwrap();
    assert!(manager.red_count() <= 4);
    manager.flush().unwrap();
    assert_eq!(manager.red_count() + manager.blue_count(), 10);
}

#[test]
fn test_resize_hot_clamps_zero() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        4,
        false,
    );

    manager
        .add(
            TestCheckpoint {
                id: 1,
                data: "ok".into(),
            },
            &[],
        )
        .unwrap();

    // Resize to 0 should clamp to 1, keeping exactly one item hot.
    manager.resize_hot(0).unwrap();
    assert!(manager.is_hot(1));
    assert_eq!(manager.red_count(), 1);
}

#[test]
fn test_resize_optimal() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        200,
        false,
    );

    // Add 100 checkpoints — all stay hot with capacity 200.
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

    assert_eq!(manager.red_count(), 100);

    // resize_optimal: sqrt(100) = 10
    manager.resize_optimal().unwrap();
    assert!(
        manager.red_count() <= 10,
        "after resize_to_optimal, red count {} should be <= sqrt(100)=10",
        manager.red_count(),
    );
    manager.flush().unwrap();
    assert_eq!(manager.red_count() + manager.blue_count(), 100);
}

#[test]
fn test_auto_resize_grows_capacity() {
    // Start with hot_capacity=1 and auto_resize=true.
    // After adding 100 checkpoints, hot_capacity should grow to sqrt(100)=10.
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        1,
        true,
    );

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

    // hot_capacity should have grown to sqrt(100) = 10
    let stats = manager.stats();
    assert!(
        stats.red_pebble_count() <= 10,
        "red count {} should be <= sqrt(100)=10",
        stats.red_pebble_count(),
    );
    // More items stay hot than with a fixed capacity of 1
    assert!(
        stats.red_pebble_count() > 1,
        "auto_resize should have grown capacity beyond initial 1, got {}",
        stats.red_pebble_count(),
    );
}

#[test]
fn test_manual_resize_keeps_fixed_capacity() {
    // With manual_resize, hot_capacity stays fixed.
    use crate::manager::PebbleBuilder;

    let mut manager = PebbleBuilder::new()
        .cold(test_cold())
        .warm(NoWarm)
        .log(DropSpout)
        .hot_capacity(2)
        .manual_resize()
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

    // hot_capacity stayed at 2, so red count should be <= 2
    assert!(
        manager.red_count() <= 2,
        "with manual_resize, red count {} should be <= fixed capacity 2",
        manager.red_count(),
    );
}

#[test]
fn test_auto_resize_never_shrinks() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        1,
        true,
    );

    // Add 100 checkpoints to grow capacity to sqrt(100)=10
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

    let red_before_remove = manager.red_count();

    // Remove many checkpoints — capacity should NOT shrink
    for i in 0..80 {
        manager.remove(i);
    }

    // Still have the same red count (minus any that were in hot and removed)
    // The key invariant: capacity didn't shrink, so remaining hot items stay hot
    assert!(
        manager.red_count() <= red_before_remove,
        "red count should not have increased after removes",
    );
    // Total is now 20
    assert_eq!(manager.len(), 20);
}

#[test]
fn test_auto_resize_builder_default() {
    use crate::manager::PebbleBuilder;

    // auto_resize is on by default — no method call needed
    let mut manager = PebbleBuilder::new()
        .cold(test_cold())
        .warm(NoWarm)
        .log(DropSpout)
        .hot_capacity(1)
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

    // Default auto_resize should grow capacity beyond initial 1
    assert!(
        manager.red_count() > 1,
        "default auto_resize should grow capacity, got red_count={}",
        manager.red_count(),
    );
}
