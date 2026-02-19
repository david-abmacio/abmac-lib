//! Eviction candidate selection and rebuild ordering for the DAG.

use alloc::vec::Vec;
use core::hash::Hash;
use hashbrown::{HashMap, HashSet};

use super::ComputationDAG;

impl<T: Copy + Eq + Hash + core::fmt::Debug> ComputationDAG<T> {
    /// Build rebuild order for nodes not in `available`, sorted by depth.
    pub fn rebuild_order(&self, target: T, available: &HashSet<T>) -> Vec<T> {
        // BFS to find all required nodes not in available set
        let mut required = HashSet::new();
        let mut to_visit = Vec::new();
        to_visit.push(target);

        while let Some(node_id) = to_visit.pop() {
            if required.contains(&node_id) || available.contains(&node_id) {
                continue;
            }
            required.insert(node_id);

            if let Some(node) = self.nodes.get(&node_id) {
                for dep in &node.dependencies {
                    if !available.contains(dep) && !required.contains(dep) {
                        to_visit.push(*dep);
                    }
                }
            }
        }

        // Sort by rebuild_depth (ascending) for correct dependency order
        let mut order: Vec<T> = required.into_iter().collect();
        order.sort_by_key(|id| self.nodes.get(id).map(|n| n.rebuild_depth).unwrap_or(0));

        order
    }

    /// Get nodes with no active dependents, sorted by access frequency.
    pub fn get_eviction_candidates<V>(&self, active_nodes: &HashMap<T, V>) -> Vec<T> {
        let mut candidates = Vec::new();

        for active_id in active_nodes.keys() {
            if let Some(node) = self.nodes.get(active_id) {
                let has_active_dependents = node
                    .dependents
                    .iter()
                    .any(|dep| active_nodes.contains_key(dep));

                if !has_active_dependents {
                    candidates.push(*active_id);
                }
            }
        }

        // Sort by access frequency (LRU) and creation time
        candidates.sort_by_key(|id| match self.nodes.get(id) {
            Some(node) => (node.access_frequency, node.creation_time),
            None => (u64::MAX, u64::MAX),
        });

        candidates
    }
}
