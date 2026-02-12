//! Tests for pebbling strategies.

use hashbrown::HashMap;

use crate::dag::{ComputationDAG, DAGStats};
use crate::strategy::{DAGStrategy, Strategy, TreeStrategy};

#[test]
fn test_eviction_count() {
    let strategy = Strategy::default();

    // 100 total nodes, 20 in fast memory
    // sqrt(100) = 10, so should evict 10
    assert_eq!(strategy.get_eviction_count(20, 100), 10);

    // Already at optimal
    assert_eq!(strategy.get_eviction_count(10, 100), 0);

    // Below optimal
    assert_eq!(strategy.get_eviction_count(5, 100), 0);
}

#[test]
fn test_recommended_memory_size() {
    let stats = DAGStats {
        total_nodes: 1000,
        root_nodes: 1,
        leaf_nodes: 100,
        max_depth: 10,
        average_fanout: 1.5,
    };

    let tree_strategy = Strategy::Tree(TreeStrategy::new());
    let dag_strategy = Strategy::DAG(DAGStrategy::default());

    // Tree uses sqrt(leaves), DAG uses sqrt(total)
    let tree_size = tree_strategy.recommended_hot_capacity(&stats);
    let dag_size = dag_strategy.recommended_hot_capacity(&stats);

    assert!(tree_size >= 16); // Minimum bound
    assert!(dag_size >= 32); // Minimum bound
}

#[test]
fn test_tree_strategy_postorder_eviction() {
    // postorder mode: leaf_first=false, postorder_priority=true
    // Should sort candidates by critical path length (shortest first).
    let strategy = TreeStrategy {
        postorder_priority: true,
        leaf_first: false,
    };

    //     1
    //    / \
    //   2   3
    //   |
    //   4
    // Critical paths: 4=0, 3=0, 2=1, 1=2
    let mut dag: ComputationDAG<u64> = ComputationDAG::new();
    dag.add_node(1, &[]).unwrap();
    dag.add_node(2, &[1]).unwrap();
    dag.add_node(3, &[1]).unwrap();
    dag.add_node(4, &[2]).unwrap();

    // All nodes active
    let mut active: HashMap<u64, ()> = HashMap::new();
    for id in 1..=4u64 {
        active.insert(id, ());
    }

    // Evict 2: candidates without active dependents are 3 (cp=0) and 4 (cp=0).
    // Both have the same critical path; either order is valid.
    let candidates = strategy.select_eviction_candidates(&active, &dag, 2);
    assert_eq!(candidates.len(), 2);
    assert!(candidates.contains(&3));
    assert!(candidates.contains(&4));
    // Node 1 and 2 should NOT be evicted (they have active dependents)
    assert!(!candidates.contains(&1));
    assert!(!candidates.contains(&2));
}

#[test]
fn test_tree_strategy_leaf_first_eviction() {
    // Default leaf_first mode: sorts by fewest dependents.
    let strategy = TreeStrategy::new();

    //   1
    //  / \
    // 2   3
    //     |
    //     4
    let mut dag: ComputationDAG<u64> = ComputationDAG::new();
    dag.add_node(1, &[]).unwrap();
    dag.add_node(2, &[1]).unwrap();
    dag.add_node(3, &[1]).unwrap();
    dag.add_node(4, &[3]).unwrap();

    let mut active: HashMap<u64, ()> = HashMap::new();
    for id in 1..=4u64 {
        active.insert(id, ());
    }

    // Candidates: 2 (0 dependents) and 4 (0 dependents).
    // Evict 1: should pick one of the leaves.
    let candidates = strategy.select_eviction_candidates(&active, &dag, 1);
    assert_eq!(candidates.len(), 1);
    assert!(candidates[0] == 2 || candidates[0] == 4);
}

#[test]
fn test_dag_strategy_eviction_ordering() {
    // DAG strategy with Hybrid mode: critical_path, then computation_cost, then LRU.
    //
    // Build a DAG with multiple leaf candidates that differ in critical path:
    //
    //   1 (root)
    //  /|\
    // 2 3 4  (all leaves, all candidates)
    // |
    // 5      (leaf, candidate)
    //
    // Critical paths: 5=0, 2=1 (has dependent 5), 3=0, 4=0
    // But 2 has an active dependent (5), so 2 is NOT a candidate.
    // Candidates: 3(cp=0), 4(cp=0), 5(cp=0)
    // All same critical path; tie-break by computation_cost (dep count):
    //   3: cost=1 (depends on [1]), 4: cost=1, 5: cost=1 (depends on [2])
    // Then by access_frequency (all 1).
    //
    // To get different critical paths, use a deeper structure:
    //
    //   1 (root)
    //  / \
    // 2   3
    // |
    // 4
    // |
    // 5
    //
    // Candidates without active dependents: 3 and 5.
    // Critical paths: 5=0, 4=1, 3=0, 2=2, 1=3
    // Candidates: 3(cp=0, cost=1), 5(cp=0, cost=1) — same ordering.
    //
    // Better: give different depths by adding more branches.
    //
    //   1 (root)
    //  / \
    // 2   3
    // |   |
    // 4   5
    //
    // Candidates: 4 and 5 (leaves with no dependents).
    // Critical paths: 4=0, 5=0, 2=1, 3=1, 1=2
    // Same critical path. Tie-break: cost (both 1), then access_frequency.
    // Access node 5 to make its frequency higher → 4 should be evicted first.
    let strategy = DAGStrategy::default();

    let mut dag: ComputationDAG<u64> = ComputationDAG::new();
    dag.add_node(1, &[]).unwrap();
    dag.add_node(2, &[1]).unwrap();
    dag.add_node(3, &[1]).unwrap();
    dag.add_node(4, &[2]).unwrap();
    dag.add_node(5, &[3]).unwrap();

    // Bump access frequency on node 5 so it sorts after node 4
    dag.mark_accessed(5);
    dag.mark_accessed(5);

    let mut active: HashMap<u64, ()> = HashMap::new();
    for id in 1..=5u64 {
        active.insert(id, ());
    }

    // Candidates: 4 and 5 (no active dependents).
    // Both cp=0, cost=1. Node 4 has freq=1, node 5 has freq=3.
    // Hybrid sorts by (cp, cost, freq) ascending → 4 first, then 5.
    let candidates = strategy.select_eviction_candidates(&active, &dag, 2);
    assert_eq!(candidates.len(), 2);
    assert_eq!(
        candidates[0], 4,
        "node 4 should be evicted first (lower access frequency)"
    );
    assert_eq!(
        candidates[1], 5,
        "node 5 should be evicted second (higher access frequency)"
    );
}
