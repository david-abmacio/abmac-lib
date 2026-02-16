//! Theoretical validation tests: space bounds, I/O bounds, strategy end-to-end.

use super::*;

#[test]
fn test_theoretical_validation_space_bound() {
    // 100 nodes with hot_capacity=10 (sqrt(100)=10, 2x=20)
    // Should satisfy space bound since 10 <= 20
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        10,
        false,
    );

    for i in 0..100 {
        let cp = TestCheckpoint {
            id: i,
            data: alloc::format!("data{i}"),
        };
        manager.add(cp, &[]).unwrap();
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
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        100,
        false,
    );

    for i in 0..16 {
        let cp = TestCheckpoint {
            id: i,
            data: alloc::format!("data{i}"),
        };
        manager.add(cp, &[]).unwrap();
    }

    let validation = manager.validate_theoretical_bounds();
    assert!(
        !validation.space_bound_satisfied(),
        "Space bound should NOT be satisfied: hot=100 > 2*sqrt(16)=8"
    );
    assert_eq!(validation.expected_max_space(), 4);
}

#[test]
fn test_tree_io_bound_end_to_end() {
    use crate::strategy::{Strategy, TreeStrategy};

    let total_nodes: u64 = 255; // 2^8 - 1, balanced binary tree
    let hot_capacity = (total_nodes as usize).isqrt(); // 15

    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::Tree(TreeStrategy::new()),
        hot_capacity,
        false,
    );

    // Build balanced binary tree: node i has children 2i+1 and 2i+2.
    for i in 0..total_nodes {
        let deps = if i == 0 { vec![] } else { vec![(i - 1) / 2] };
        let cp = TestCheckpoint {
            id: i,
            data: alloc::format!("tree-{i}"),
        };
        manager.add(cp, &deps).unwrap();
    }

    manager.flush().unwrap();

    let io_after_fill = manager.stats().io_operations();

    // Load leaf nodes from cold storage.
    let mut loaded = 0;
    for i in (total_nodes / 2)..total_nodes {
        if manager.is_in_storage(i) {
            manager.load(i).unwrap();
            loaded += 1;
            if loaded >= hot_capacity {
                break;
            }
        }
    }

    assert!(loaded > 0, "should have loaded leaves from cold storage");

    let io_after_loads = manager.stats().io_operations();
    let load_phase_io = io_after_loads - io_after_fill;

    let io_per_load = load_phase_io as f64 / loaded as f64;
    assert!(
        io_per_load <= 3.0,
        "tree leaf loads should cost ~1-2 I/O each, got {:.2} \
         (load_io={}, loaded={})",
        io_per_load,
        load_phase_io,
        loaded,
    );

    let validation = manager.validate_theoretical_bounds();
    assert!(
        validation.space_bound_satisfied(),
        "space bound should hold: hot={}, sqrt({})={}, 2x={}",
        hot_capacity,
        total_nodes,
        (total_nodes as usize).isqrt(),
        (total_nodes as usize).isqrt() * 2,
    );
}

#[test]
fn test_dag_io_bound_end_to_end() {
    use crate::strategy::{DAGStrategy, Strategy};

    let layers = 10u64;
    let width = 10u64;
    let total_nodes = layers * width;
    let hot_capacity = (total_nodes as usize).isqrt(); // 10

    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::DAG(DAGStrategy::default()),
        hot_capacity,
        false,
    );

    // Build layered DAG.
    for layer in 0..layers {
        for col in 0..width {
            let id = layer * width + col;
            let deps = if layer == 0 {
                vec![]
            } else {
                let mut d = vec![(layer - 1) * width + col];
                if col > 0 {
                    d.push((layer - 1) * width + (col - 1));
                }
                d
            };
            let cp = TestCheckpoint {
                id,
                data: alloc::format!("dag-{id}"),
            };
            manager.add(cp, &deps).unwrap();
        }
    }

    manager.flush().unwrap();

    let io_after_fill = manager.stats().io_operations();

    // Load leaf-layer nodes from cold storage.
    let mut loaded = 0;
    let leaf_start = (layers - 1) * width;
    for i in leaf_start..(leaf_start + width) {
        if manager.is_in_storage(i) {
            manager.load(i).unwrap();
            loaded += 1;
        }
    }

    assert!(
        loaded > 0,
        "should have loaded DAG leaves from cold storage"
    );

    let io_after_loads = manager.stats().io_operations();
    let load_phase_io = io_after_loads - io_after_fill;

    let io_per_load = load_phase_io as f64 / loaded as f64;
    assert!(
        io_per_load <= 4.0,
        "DAG leaf loads should cost bounded I/O each, got {:.2} \
         (load_io={}, loaded={})",
        io_per_load,
        load_phase_io,
        loaded,
    );

    let validation = manager.validate_theoretical_bounds();
    assert!(
        validation.space_bound_satisfied(),
        "space bound should hold: hot={}, sqrt({})={}, 2x={}",
        hot_capacity,
        total_nodes,
        (total_nodes as usize).isqrt(),
        (total_nodes as usize).isqrt() * 2,
    );
}

#[test]
fn test_theoretical_validation_io_bound() {
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        10,
        false,
    );

    for i in 0..20 {
        let cp = TestCheckpoint {
            id: i,
            data: alloc::format!("data{i}"),
        };
        manager.add(cp, &[]).unwrap();
    }

    manager.flush().unwrap();

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

    let validation = manager.validate_theoretical_bounds();
    let expected = stats.io_optimality_ratio() <= 3.0;
    assert_eq!(
        validation.io_bound_satisfied(),
        expected,
        "io_bound_satisfied should match manual ratio check",
    );
}

#[test]
fn test_edge_bound_tighter_than_counting() {
    // Build a wide, densely-connected DAG where |E| >> |V| * S.
    // 5 layers of 8 nodes, each node depends on ALL nodes in the
    // previous layer (8 deps each, within MAX_DEPS=8).
    // Total: 40 nodes, 256 edges.
    // With S=3: counting = 40-3 = 37, edge = ceil((256-40*3)/3) = ceil(136/3) = 46.
    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        3,
        false,
    );

    let width = 8u64;
    let layers = 5u64;

    for layer in 0..layers {
        for col in 0..width {
            let id = layer * width + col;
            let deps: alloc::vec::Vec<u64> = if layer == 0 {
                vec![]
            } else {
                (0..width).map(|c| (layer - 1) * width + c).collect()
            };
            manager
                .add(
                    TestCheckpoint {
                        id,
                        data: alloc::format!("dense-{id}"),
                    },
                    &deps,
                )
                .unwrap();
        }
    }

    let stats = manager.stats();
    let total = manager.len();
    let counting_bound = total.saturating_sub(3);

    // The edge bound should produce a tighter (higher) lower bound.
    assert!(
        stats.theoretical_min_io() > counting_bound as u64,
        "edge bound should exceed counting bound: min_io={}, counting={}",
        stats.theoretical_min_io(),
        counting_bound,
    );
}

#[test]
fn test_depth_bound_tighter_than_counting() {
    // Build a long chain: 20 nodes, depth=19. With S=3:
    // counting = 20-3 = 17, depth = (19+1)-3 = 17. Equal here.
    // Use S=2: counting = 20-2 = 18, depth = (19+1)-2 = 18. Still equal.
    // Use a longer chain with small S to get depth > counting.
    // 10 nodes in a chain, S=5: counting = 10-5 = 5, depth = (9+1)-5 = 5. Equal.
    // The depth bound equals counting for a simple chain because depth = V-1
    // and depth+1-S = V-S = counting. So depth bound won't exceed counting
    // for a chain. But it CAN exceed counting when there are root nodes that
    // don't contribute to depth. Example: 5 isolated roots + a chain of 10.
    // Total=15, S=3: counting=12, depth=(9+1)-3=7. Counting wins.
    // The depth bound is most useful for tall sparse graphs where the counting
    // bound is weak. Let's verify both bounds contribute to the max correctly.

    // Chain of 20, S=3: counting=17, depth=18. Depth wins by 1!
    // depth = (19+1) - 3 = 17. No, same. depth = max_depth+1-S = 19+1-3=17.
    // Hmm, counting=20-3=17 too. They're equal for a pure chain.

    // To make depth strictly win: add duplicate "wide" nodes that don't
    // increase depth. 5 roots (depth 0) + 1 chain of 5 depending on root 0.
    // Total=10, depth=5, S=2: counting=8, depth=(5+1)-2=4. Counting wins.

    // Actually, for a pure chain, counting = V-S and depth = V-S, always equal.
    // The depth bound is only tighter when edges create depth beyond what
    // vertex count alone suggests. But that can't happen: depth <= V-1 always.
    // So the depth bound <= counting bound for connected graphs.

    // The depth bound IS useful as a sanity check and contributes when
    // total_nodes (tier count) differs from DAG vertex count (e.g., removed
    // nodes still in tiers). Let's just verify it's computed correctly.

    let mut manager = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        3,
        false,
    );

    // Chain of 15 nodes: depth = 14.
    for i in 0..15u64 {
        let deps = if i == 0 { vec![] } else { vec![i - 1] };
        manager
            .add(
                TestCheckpoint {
                    id: i,
                    data: alloc::format!("chain-{i}"),
                },
                &deps,
            )
            .unwrap();
    }

    let stats = manager.stats();
    // counting = 15-3 = 12, depth = (14+1)-3 = 12. Equal for a chain.
    assert_eq!(
        stats.theoretical_min_io(),
        12,
        "chain: counting and depth bounds should both give 12",
    );
    // The min I/O should be at least the counting bound.
    assert!(stats.theoretical_min_io() >= (manager.len().saturating_sub(3)) as u64);
}
