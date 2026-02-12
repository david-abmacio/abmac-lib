//! Tests for the computation DAG.

use alloc::vec;
use alloc::vec::Vec;
use hashbrown::{HashMap, HashSet};

use crate::dag::{ComputationDAG, DAGError};

#[test]
fn test_dag_basic() {
    let mut dag: ComputationDAG<u64> = ComputationDAG::new();

    dag.add_node(1, &[]).unwrap();
    dag.add_node(2, &[1]).unwrap();
    dag.add_node(3, &[1]).unwrap();
    dag.add_node(4, &[2, 3]).unwrap();

    assert_eq!(dag.len(), 4);
    assert!(!dag.is_empty());

    let stats = dag.stats();
    assert_eq!(stats.total_nodes, 4);
    assert_eq!(stats.root_nodes, 1);
    assert_eq!(stats.leaf_nodes, 1);
}

#[test]
fn test_dag_dependencies() {
    let mut dag: ComputationDAG<u64> = ComputationDAG::new();

    dag.add_node(1, &[]).unwrap();
    dag.add_node(2, &[1]).unwrap();
    dag.add_node(3, &[2]).unwrap();

    // rebuild_order with nothing available returns the target + all transitive deps
    let order = dag.rebuild_order(3, &HashSet::new());
    assert!(order.contains(&1));
    assert!(order.contains(&2));
    assert!(order.contains(&3));
}

#[test]
fn test_eviction_candidates() {
    let mut dag: ComputationDAG<u64> = ComputationDAG::new();

    dag.add_node(1, &[]).unwrap();
    dag.add_node(2, &[1]).unwrap();
    dag.add_node(3, &[1]).unwrap();

    let mut active = HashMap::new();
    active.insert(1u64, ());
    active.insert(2u64, ());
    active.insert(3u64, ());

    // Nodes 2 and 3 have no active dependents, so they're candidates
    let candidates = dag.get_eviction_candidates(&active);
    assert!(candidates.contains(&2));
    assert!(candidates.contains(&3));
    // Node 1 has active dependents, so it's not a candidate
    assert!(!candidates.contains(&1));
}

#[test]
fn test_access_frequency() {
    let mut dag: ComputationDAG<u64> = ComputationDAG::new();

    dag.add_node(1, &[]).unwrap();
    assert_eq!(dag.get_node(1).unwrap().access_frequency, 1); // initial

    dag.mark_accessed(1);
    dag.mark_accessed(1);

    let node = dag.get_node(1).unwrap();
    assert_eq!(node.access_frequency, 3); // 1 initial + 2 marks
}

#[test]
fn test_rebuild_depth() {
    let mut dag: ComputationDAG<u64> = ComputationDAG::new();

    // Build a chain: 1 -> 2 -> 3 -> 4
    dag.add_node(1, &[]).unwrap();
    dag.add_node(2, &[1]).unwrap();
    dag.add_node(3, &[2]).unwrap();
    dag.add_node(4, &[3]).unwrap();

    assert_eq!(dag.get_node(1).unwrap().rebuild_depth, 0);
    assert_eq!(dag.get_node(2).unwrap().rebuild_depth, 1);
    assert_eq!(dag.get_node(3).unwrap().rebuild_depth, 2);
    assert_eq!(dag.get_node(4).unwrap().rebuild_depth, 3);
}

#[test]
fn test_max_dependency_width() {
    let mut dag: ComputationDAG<u64> = ComputationDAG::new();

    dag.add_node(1, &[]).unwrap();
    dag.add_node(2, &[]).unwrap();
    dag.add_node(3, &[]).unwrap();
    dag.add_node(4, &[1, 2, 3]).unwrap(); // 3 dependencies

    assert_eq!(dag.max_dependency_width(), 3);
}

#[test]
fn test_rebuild_order() {
    let mut dag: ComputationDAG<u64> = ComputationDAG::new();

    // Chain: 1 -> 2 -> 3 -> 4
    dag.add_node(1, &[]).unwrap();
    dag.add_node(2, &[1]).unwrap();
    dag.add_node(3, &[2]).unwrap();
    dag.add_node(4, &[3]).unwrap();

    // Nothing available - need everything
    let available = HashSet::new();
    let order = dag.rebuild_order(4, &available);
    assert_eq!(order, vec![1, 2, 3, 4]);

    // Node 1 already available
    let mut available = HashSet::new();
    available.insert(1u64);
    let order = dag.rebuild_order(4, &available);
    assert_eq!(order, vec![2, 3, 4]);

    // Nodes 1 and 2 available
    available.insert(2u64);
    let order = dag.rebuild_order(4, &available);
    assert_eq!(order, vec![3, 4]);
}

#[test]
fn test_rebuild_order_diamond() {
    let mut dag: ComputationDAG<u64> = ComputationDAG::new();

    // Diamond: 1 -> {2, 3} -> 4
    dag.add_node(1, &[]).unwrap();
    dag.add_node(2, &[1]).unwrap();
    dag.add_node(3, &[1]).unwrap();
    dag.add_node(4, &[2, 3]).unwrap();

    let available = HashSet::new();
    let order = dag.rebuild_order(4, &available);

    // Order should have 1 first, then 2 and 3 (either order), then 4
    assert_eq!(order[0], 1);
    assert!(order[1] == 2 || order[1] == 3);
    assert!(order[2] == 2 || order[2] == 3);
    assert_ne!(order[1], order[2]);
    assert_eq!(order[3], 4);
}

// Validation tests

#[test]
fn test_missing_dependency_error() {
    let mut dag: ComputationDAG<u64> = ComputationDAG::new();

    // Try to add node with non-existent dependency
    let result = dag.add_node(2, &[1]);
    assert!(matches!(result, Err(DAGError::MissingDependency { .. })));
    if let Err(DAGError::MissingDependency { dep_id }) = result {
        assert_eq!(dep_id, "1");
    }
}

#[test]
fn test_self_dependency_error() {
    let mut dag: ComputationDAG<u64> = ComputationDAG::new();

    // Try to add node that depends on itself
    let result = dag.add_node(1, &[1]);
    assert!(matches!(result, Err(DAGError::SelfDependency { .. })));
    if let Err(DAGError::SelfDependency { node_id }) = result {
        assert_eq!(node_id, "1");
    }
}

#[test]
fn test_add_node_unchecked() {
    let mut dag: ComputationDAG<u64> = ComputationDAG::new();

    // add_node_unchecked allows missing dependencies (for unordered loading)
    dag.add_node_unchecked(2, &[1]); // 1 doesn't exist yet
    dag.add_node_unchecked(1, &[]); // Add it later

    // Must call repair() after unchecked additions
    dag.repair().unwrap();

    assert!(dag.contains(1));
    assert!(dag.contains(2));

    // Verify repair fixed the structure
    assert_eq!(dag.get_node(1).unwrap().rebuild_depth, 0);
    assert_eq!(dag.get_node(2).unwrap().rebuild_depth, 1);
    assert!(dag.get_node(1).unwrap().dependents.contains(&2));
}

// Error path tests

#[test]
fn test_cycle_detected_via_unchecked_repair() {
    let mut dag: ComputationDAG<u64> = ComputationDAG::new();

    // Create a cycle: 1 -> 2 -> 3 -> 1
    dag.add_node_unchecked(1, &[3]);
    dag.add_node_unchecked(2, &[1]);
    dag.add_node_unchecked(3, &[2]);

    let result = dag.repair();
    assert!(matches!(result, Err(DAGError::CycleDetected { .. })));
}

#[test]
fn test_node_exists_error() {
    let mut dag: ComputationDAG<u64> = ComputationDAG::new();

    dag.add_node(1, &[]).unwrap();
    let result = dag.add_node(1, &[]);
    assert!(matches!(result, Err(DAGError::NodeExists { .. })));
    if let Err(DAGError::NodeExists { node_id }) = result {
        assert_eq!(node_id, "1");
    }
}

// Incremental critical path tests

#[test]
fn test_critical_paths_incremental() {
    // Build a diamond node-by-node, verifying the cache after each add.
    //     1
    //    / \
    //   2   3
    //    \ /
    //     4
    let mut dag: ComputationDAG<u64> = ComputationDAG::new();

    dag.add_node(1, &[]).unwrap();
    assert_eq!(dag.critical_path(1), Some(0)); // lone root is also a leaf

    dag.add_node(2, &[1]).unwrap();
    assert_eq!(dag.critical_path(2), Some(0)); // new leaf
    assert_eq!(dag.critical_path(1), Some(1)); // propagated up

    dag.add_node(3, &[1]).unwrap();
    assert_eq!(dag.critical_path(3), Some(0));
    assert_eq!(dag.critical_path(1), Some(1)); // unchanged (max still 1)

    dag.add_node(4, &[2, 3]).unwrap();
    assert_eq!(dag.critical_path(4), Some(0)); // new leaf
    assert_eq!(dag.critical_path(2), Some(1));
    assert_eq!(dag.critical_path(3), Some(1));
    assert_eq!(dag.critical_path(1), Some(2)); // propagated deeper

    // Also verify rebuild depths for the diamond
    assert_eq!(dag.get_node(1).unwrap().rebuild_depth, 0);
    assert_eq!(dag.get_node(2).unwrap().rebuild_depth, 1);
    assert_eq!(dag.get_node(3).unwrap().rebuild_depth, 1);
    assert_eq!(dag.get_node(4).unwrap().rebuild_depth, 2);
}

#[test]
fn test_critical_paths_deep_chain() {
    // Chain of 5: 1 -> 2 -> 3 -> 4 -> 5
    let mut dag: ComputationDAG<u64> = ComputationDAG::new();

    for i in 1..=5 {
        let deps: Vec<u64> = if i == 1 { vec![] } else { vec![i - 1] };
        dag.add_node(i, &deps).unwrap();
    }

    // Leaf (5) has depth 0, root (1) has depth 4
    for i in 1..=5u64 {
        assert_eq!(dag.critical_path(i), Some((5 - i) as usize));
    }

    // Verify incremental result matches per-node lookups
    let paths = dag.critical_paths_ref();
    for i in 1..=5u64 {
        assert_eq!(paths.get(&i).copied(), dag.critical_path(i));
    }
}

#[test]
fn test_critical_paths_after_repair() {
    // Build with unchecked (out-of-order) adds, then repair.
    // Diamond: 1 -> {2, 3} -> 4
    let mut dag: ComputationDAG<u64> = ComputationDAG::new();

    dag.add_node_unchecked(4, &[2, 3]);
    dag.add_node_unchecked(2, &[1]);
    dag.add_node_unchecked(3, &[1]);
    dag.add_node_unchecked(1, &[]);

    dag.repair().unwrap();

    assert_eq!(dag.critical_path(4), Some(0));
    assert_eq!(dag.critical_path(2), Some(1));
    assert_eq!(dag.critical_path(3), Some(1));
    assert_eq!(dag.critical_path(1), Some(2));
}
