//! DAG validation and depth computation.

use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;
use core::hash::Hash;
use hashbrown::HashMap;

use super::{ComputationDAG, DAGError};

/// DFS node state for cycle detection.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum DfsColor {
    /// Unvisited node.
    White = 0,
    /// Node is on the current DFS path (in progress).
    Gray = 1,
    /// Fully processed node.
    Black = 2,
}

impl<T: Copy + Eq + Hash + core::fmt::Debug> ComputationDAG<T> {
    /// Validate the DAG is acyclic.
    pub fn validate_acyclic(&self) -> Result<(), DAGError> {
        let mut colors: HashMap<T, DfsColor> = HashMap::new();

        for &start in self.nodes.keys() {
            if colors.get(&start).copied().unwrap_or(DfsColor::White) != DfsColor::White {
                continue;
            }

            let mut stack: Vec<(T, usize)> = vec![(start, 0)];
            colors.insert(start, DfsColor::Gray);

            while let Some((node, idx)) = stack.pop() {
                let deps = self
                    .nodes
                    .get(&node)
                    .map(|n| &n.dependencies[..])
                    .unwrap_or(&[]);

                if idx >= deps.len() {
                    colors.insert(node, DfsColor::Black);
                    continue;
                }

                stack.push((node, idx + 1));
                let dep = deps[idx];

                match colors.get(&dep).copied().unwrap_or(DfsColor::White) {
                    DfsColor::White => {
                        colors.insert(dep, DfsColor::Gray);
                        stack.push((dep, 0));
                    }
                    DfsColor::Gray => {
                        return Err(DAGError::CycleDetected {
                            node_id: alloc::format!("{:?}", dep),
                        });
                    }
                    DfsColor::Black => {}
                }
            }
        }

        Ok(())
    }

    /// Kahn's algorithm: O(V+E) topological sort with in-degree counters.
    pub(crate) fn recalculate_rebuild_depths(&mut self) {
        let mut in_degree: HashMap<T, usize> = HashMap::new();
        for (id, node) in &self.nodes {
            in_degree.entry(*id).or_insert(0);
            for &dep in &node.dependents {
                *in_degree.entry(dep).or_insert(0) += 1;
            }
        }

        // Seed queue with roots (in-degree 0)
        let mut queue: VecDeque<T> = in_degree
            .iter()
            .filter(|&(_, deg)| *deg == 0)
            .map(|(&id, _)| id)
            .collect();

        // Reset all depths
        for node in self.nodes.values_mut() {
            node.rebuild_depth = 0;
        }

        while let Some(id) = queue.pop_front() {
            let Some(node) = self.nodes.get(&id) else {
                continue;
            };

            let max_dep_depth = node
                .dependencies
                .iter()
                .filter_map(|d| self.nodes.get(d))
                .map(|d| d.rebuild_depth)
                .max();

            let depth = match max_dep_depth {
                Some(d) => d.saturating_add(1),
                None => 0,
            };

            // Collect dependents before mutable borrow
            let dependents: Vec<T> = node.dependents.to_vec();

            if let Some(n) = self.nodes.get_mut(&id) {
                n.rebuild_depth = depth;
            }

            for dep in dependents {
                if let Some(deg) = in_degree.get_mut(&dep) {
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push_back(dep);
                    }
                }
            }
        }
    }

    /// Maximum depth (longest path) in the DAG: O(V).
    ///
    /// Uses the `rebuild_depth` values maintained by
    /// [`recalculate_rebuild_depths`](Self::recalculate_rebuild_depths).
    pub(crate) fn calculate_max_depth(&self) -> usize {
        self.nodes
            .values()
            .map(|n| n.rebuild_depth as usize)
            .max()
            .unwrap_or(0)
    }
}
