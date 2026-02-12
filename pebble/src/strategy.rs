//! Pebbling strategies for checkpoint management.
//!
//! Both strategies use O(sqrt(T)) space via the Red-Blue Pebble Game model.
//!
//! - [`TreeStrategy`]: Leaf-first eviction achieves a 2-approximation of optimal I/O
//!   for tree-structured computations (Gleinig & Hoefler 2022).
//! - [`DAGStrategy`]: Critical-path eviction for general DAGs (3x I/O budget;
//!   optimal scheduling is PSPACE-hard so the bound is unproven).
//!
//! References:
//! - Hong & Kung, "I/O complexity: The red-blue pebble game" (STOC 1981)
//! - Gleinig & Hoefler, "The red-blue pebble game on trees and DAGs with large input" (Euro-Par 2022)

use alloc::vec::Vec;
use core::hash::Hash;

/// Minimum recommended hot capacity for tree strategies.
const TREE_MIN_HOT_CAPACITY: usize = 16;

/// Multiplier for tree strategy recommended capacity: sqrt(leaves) * this.
const TREE_CAPACITY_MULTIPLIER: usize = 2;

/// Minimum recommended hot capacity for DAG strategies.
const DAG_MIN_HOT_CAPACITY: usize = 32;
use hashbrown::HashMap;

use crate::dag::{ComputationDAG, DAGStats};

/// Pebbling strategy.
#[derive(Debug, Clone)]
pub enum Strategy {
    Tree(TreeStrategy),
    DAG(DAGStrategy),
}

impl Strategy {
    pub fn select_eviction_candidates<T, V>(
        &self,
        active_nodes: &HashMap<T, V>,
        dag: &ComputationDAG<T>,
        target_eviction_count: usize,
    ) -> Vec<T>
    where
        T: Copy + Eq + Hash + core::fmt::Debug,
    {
        match self {
            Strategy::Tree(strategy) => {
                strategy.select_eviction_candidates(active_nodes, dag, target_eviction_count)
            }
            Strategy::DAG(strategy) => {
                strategy.select_eviction_candidates(active_nodes, dag, target_eviction_count)
            }
        }
    }

    pub fn get_eviction_count(&self, red_count: usize, total_count: usize) -> usize {
        let optimal_red_count = total_count.isqrt();
        red_count.saturating_sub(optimal_red_count)
    }

    pub fn recommended_hot_capacity(&self, dag_stats: &DAGStats) -> usize {
        match self {
            Strategy::Tree(strategy) => strategy.recommended_hot_capacity(dag_stats),
            Strategy::DAG(strategy) => strategy.recommended_hot_capacity(dag_stats),
        }
    }
}

impl Default for Strategy {
    fn default() -> Self {
        Strategy::DAG(DAGStrategy::default())
    }
}

/// Tree-optimized pebbling strategy. O(sqrt(T)) space, 2-approximate I/O for trees.
#[derive(Debug, Clone)]
pub struct TreeStrategy {
    pub postorder_priority: bool,
    pub leaf_first: bool,
}

impl Default for TreeStrategy {
    fn default() -> Self {
        Self {
            postorder_priority: true,
            leaf_first: true,
        }
    }
}

impl TreeStrategy {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn select_eviction_candidates<T, V>(
        &self,
        active_nodes: &HashMap<T, V>,
        dag: &ComputationDAG<T>,
        target_eviction_count: usize,
    ) -> Vec<T>
    where
        T: Copy + Eq + Hash + core::fmt::Debug,
    {
        let mut candidates = dag.get_eviction_candidates(active_nodes);

        if self.leaf_first {
            // For trees, prioritize leaf nodes (fewest dependents)
            candidates.sort_by_key(|node_id| {
                let node = dag.get_node(*node_id);
                match node {
                    Some(n) => (n.dependents.len(), n.access_frequency, n.creation_time),
                    None => (usize::MAX, 0, 0),
                }
            });
        } else if self.postorder_priority {
            // Use critical paths for postorder eviction
            let critical_paths = dag.critical_paths_ref();
            candidates.sort_by_key(|node_id| {
                let node = dag.get_node(*node_id);
                let critical_path = critical_paths.get(node_id).unwrap_or(&usize::MAX);
                match node {
                    Some(n) => (*critical_path, n.access_frequency, n.computation_cost),
                    None => (usize::MAX, u64::MAX, usize::MAX),
                }
            });
        }

        candidates.truncate(target_eviction_count);
        candidates
    }

    pub fn recommended_hot_capacity(&self, dag_stats: &DAGStats) -> usize {
        let sqrt_leaves = dag_stats.leaf_nodes.isqrt();
        (sqrt_leaves * TREE_CAPACITY_MULTIPLIER).max(TREE_MIN_HOT_CAPACITY)
    }
}

/// DAG eviction priority modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DAGPriorityMode {
    LeastRecentlyUsed,
    LowestComputationCost,
    FewestDependents,
    #[default]
    Hybrid,
}

/// DAG-optimized pebbling strategy. O(sqrt(T)) space.
#[derive(Debug, Clone)]
pub struct DAGStrategy {
    pub priority_mode: DAGPriorityMode,
    pub batch_processing: bool,
}

impl Default for DAGStrategy {
    fn default() -> Self {
        Self {
            priority_mode: DAGPriorityMode::Hybrid,
            batch_processing: true,
        }
    }
}

impl DAGStrategy {
    pub fn new(priority_mode: DAGPriorityMode) -> Self {
        Self {
            priority_mode,
            batch_processing: true,
        }
    }

    pub fn select_eviction_candidates<T, V>(
        &self,
        active_nodes: &HashMap<T, V>,
        dag: &ComputationDAG<T>,
        target_eviction_count: usize,
    ) -> Vec<T>
    where
        T: Copy + Eq + Hash + core::fmt::Debug,
    {
        // Borrow cached critical paths — maintained incrementally by the DAG
        let critical_paths = dag.critical_paths_ref();
        let mut candidates = dag.get_eviction_candidates(active_nodes);

        // Sort based on priority mode — all modes use critical path as primary key.
        // LowestComputationCost and Hybrid share the same scoring:
        // (critical_path, computation_cost, access_frequency).
        match self.priority_mode {
            DAGPriorityMode::LeastRecentlyUsed => {
                candidates.sort_by_key(|node_id| {
                    let node = dag.get_node(*node_id);
                    let critical_path = critical_paths.get(node_id).unwrap_or(&usize::MAX);
                    match node {
                        Some(n) => (*critical_path, n.access_frequency, n.computation_cost),
                        None => (usize::MAX, u64::MAX, usize::MAX),
                    }
                });
            }
            DAGPriorityMode::FewestDependents => {
                candidates.sort_by_key(|node_id| {
                    let node = dag.get_node(*node_id);
                    let critical_path = critical_paths.get(node_id).unwrap_or(&usize::MAX);
                    match node {
                        Some(n) => (*critical_path, n.dependents.len(), n.access_frequency),
                        None => (usize::MAX, usize::MAX, u64::MAX),
                    }
                });
            }
            DAGPriorityMode::LowestComputationCost | DAGPriorityMode::Hybrid => {
                candidates.sort_by_key(|node_id| {
                    let node = dag.get_node(*node_id);
                    let critical_path = critical_paths.get(node_id).unwrap_or(&usize::MAX);
                    match node {
                        Some(n) => (*critical_path, n.computation_cost, n.access_frequency),
                        None => (usize::MAX, usize::MAX, u64::MAX),
                    }
                });
            }
        }

        candidates.truncate(target_eviction_count);
        candidates
    }

    pub fn recommended_hot_capacity(&self, dag_stats: &DAGStats) -> usize {
        let optimal_size = dag_stats.total_nodes.isqrt();
        optimal_size.max(DAG_MIN_HOT_CAPACITY)
    }
}
