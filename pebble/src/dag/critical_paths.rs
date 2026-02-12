//! Incremental critical path maintenance.
//!
//! Critical paths (longest path to any leaf) are maintained incrementally on
//! `add_node()` and rebuilt from scratch on `repair()`.

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::hash::Hash;
use hashbrown::{HashMap, HashSet};

use super::ComputationDAG;

impl<T: Copy + Eq + Hash + core::fmt::Debug> ComputationDAG<T> {
    /// BFS backward from `starting_nodes` toward roots, updating critical paths.
    ///
    /// Each node's critical path = max(dependents' critical paths) + 1.
    /// Stops propagating a branch when the cached value doesn't increase.
    pub(crate) fn propagate_critical_paths(&mut self, starting_nodes: &[T]) {
        let mut queue: VecDeque<T> = starting_nodes.iter().copied().collect();
        let mut visited: HashSet<T> = HashSet::new();

        while let Some(node_id) = queue.pop_front() {
            if !visited.insert(node_id) {
                continue;
            }

            let Some(node) = self.nodes.get(&node_id) else {
                continue;
            };

            let new_depth = node
                .dependents
                .iter()
                .filter_map(|dep| self.critical_paths.get(dep))
                .max()
                .map(|&d| d.saturating_add(1))
                .unwrap_or(0);

            let current = self.critical_paths.entry(node_id).or_insert(0);
            if new_depth > *current {
                *current = new_depth;
                let deps: Vec<T> = node.dependencies.to_vec();
                for dep in deps {
                    if !visited.contains(&dep) {
                        queue.push_back(dep);
                    }
                }
            }
        }
    }

    /// Full O(V+E) reverse Kahn's recomputation of all critical paths.
    /// Used by `repair()` after bulk unchecked additions.
    pub(crate) fn recompute_all_critical_paths(&mut self) {
        self.critical_paths.clear();
        self.critical_paths.reserve(self.nodes.len());

        let mut out_remaining: HashMap<T, usize> = HashMap::with_capacity(self.nodes.len());
        for (&id, node) in &self.nodes {
            out_remaining.insert(id, node.dependents.len());
        }

        let mut queue: VecDeque<T> = self.leaves.iter().copied().collect();
        for &leaf in &self.leaves {
            self.critical_paths.insert(leaf, 0);
        }

        while let Some(id) = queue.pop_front() {
            let dependencies = self
                .nodes
                .get(&id)
                .map(|n| n.dependencies.as_slice())
                .unwrap_or(&[]);

            for &dep in dependencies {
                let child_depth = self.critical_paths.get(&id).copied().unwrap_or(0);
                let new_depth = child_depth.saturating_add(1);
                let current = self.critical_paths.entry(dep).or_insert(0);
                if new_depth > *current {
                    *current = new_depth;
                }

                if let Some(remaining) = out_remaining.get_mut(&dep) {
                    *remaining -= 1;
                    if *remaining == 0 {
                        queue.push_back(dep);
                    }
                }
            }
        }
    }

    /// Borrow the cached critical paths map. O(1), zero allocation.
    #[inline]
    pub fn critical_paths_ref(&self) -> &HashMap<T, usize> {
        &self.critical_paths
    }

    /// Get the critical path length for a single node. O(1).
    #[inline]
    pub fn critical_path(&self, id: T) -> Option<usize> {
        self.critical_paths.get(&id).copied()
    }
}
