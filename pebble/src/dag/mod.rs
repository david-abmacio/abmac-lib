//! Computation DAG for tracking state dependencies.
//!
//! # Thread Safety
//!
//! Not thread-safe. Use `Mutex` or `RwLock` for concurrent access.

mod critical_paths;
mod validation;

use alloc::vec::Vec;
use core::hash::Hash;
use hashbrown::{HashMap, HashSet};
verdict::display_error! {
    #[derive(Clone, PartialEq, Eq)]
    pub enum DAGError {
        #[display("missing dependency: {dep_id}")]
        MissingDependency { dep_id: alloc::string::String },

        #[display("self-dependency: {node_id}")]
        SelfDependency { node_id: alloc::string::String },

        #[display("node exists: {node_id}")]
        NodeExists { node_id: alloc::string::String },

        #[display("cycle detected: {node_id}")]
        CycleDetected { node_id: alloc::string::String },
    }
}

/// DAG node representing a compressible state.
#[derive(Debug, Clone)]
pub struct DAGNode<T> {
    pub(crate) dependencies: Vec<T>,
    pub(crate) dependents: Vec<T>,
    /// Proxy for recomputation cost. Defaults to dependency count
    /// (in-degree). Eviction strategies that use this field
    /// (`LowestComputationCost`, `Hybrid`) treat higher values as
    /// more expensive to recompute and therefore less desirable to
    /// evict.
    pub(crate) computation_cost: usize,
    pub(crate) access_frequency: u64,
    pub(crate) creation_time: u64,
    /// Longest path to any root (0 = root node).
    pub(crate) rebuild_depth: u32,
}

impl<T> DAGNode<T> {
    #[inline]
    pub fn dependencies(&self) -> &[T] {
        &self.dependencies
    }

    #[inline]
    pub fn dependents(&self) -> &[T] {
        &self.dependents
    }

    #[inline]
    pub fn computation_cost(&self) -> usize {
        self.computation_cost
    }

    #[inline]
    pub fn access_frequency(&self) -> u64 {
        self.access_frequency
    }

    #[inline]
    pub fn creation_time(&self) -> u64 {
        self.creation_time
    }

    #[inline]
    pub fn rebuild_depth(&self) -> u32 {
        self.rebuild_depth
    }
}

/// DAG structure statistics.
#[derive(Debug, Clone)]
#[must_use]
pub struct DAGStats {
    pub total_nodes: usize,
    pub root_nodes: usize,
    pub leaf_nodes: usize,
    pub max_depth: usize,
    pub average_fanout: f64,
}

/// Computation DAG for state dependencies.
#[derive(Debug)]
#[must_use = "DAG should be used after creation"]
pub struct ComputationDAG<T: Copy + Eq + Hash + core::fmt::Debug> {
    pub(crate) nodes: HashMap<T, DAGNode<T>>,
    pub(crate) roots: HashSet<T>,
    pub(crate) leaves: HashSet<T>,
    pub(crate) time_counter: u64,
    pub(crate) critical_paths: HashMap<T, usize>,
}

impl<T: Copy + Eq + Hash + core::fmt::Debug> Default for ComputationDAG<T> {
    fn default() -> Self {
        Self {
            nodes: HashMap::new(),
            roots: HashSet::new(),
            leaves: HashSet::new(),
            time_counter: 0,
            critical_paths: HashMap::new(),
        }
    }
}

impl<T: Copy + Eq + Hash + core::fmt::Debug> ComputationDAG<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a node with validated dependencies.
    pub fn add_node(&mut self, id: T, dependencies: &[T]) -> Result<(), DAGError> {
        self.add_node_impl(id, dependencies, true)
    }

    /// Add a node without validation. Call `repair()` after all additions.
    pub fn add_node_unchecked(&mut self, id: T, dependencies: &[T]) {
        let _ = self.add_node_impl(id, dependencies, false);
    }

    /// Rebuild internal structures after unchecked additions.
    pub fn repair(&mut self) -> Result<(), DAGError> {
        // Clear and rebuild dependents
        for node in self.nodes.values_mut() {
            node.dependents.clear();
        }

        // Collect all dependency relationships
        let relationships: Vec<(T, T)> = self
            .nodes
            .iter()
            .flat_map(|(&id, node)| node.dependencies.iter().map(move |&dep| (dep, id)))
            .collect();

        // Rebuild dependents
        for (dep, dependent) in relationships {
            if let Some(node) = self.nodes.get_mut(&dep) {
                node.dependents.push(dependent);
            }
        }

        // Rebuild roots and leaves
        self.roots.clear();
        self.leaves.clear();

        for (&id, node) in &self.nodes {
            if node.dependencies.is_empty() {
                self.roots.insert(id);
            }
            if node.dependents.is_empty() {
                self.leaves.insert(id);
            }
        }

        // Validate acyclicity before computing depths (prevents infinite loop)
        self.validate_acyclic()?;

        // Recalculate rebuild depths (topological order)
        self.recalculate_rebuild_depths();

        // Rebuild critical paths from scratch
        self.recompute_all_critical_paths();

        Ok(())
    }

    fn add_node_impl(&mut self, id: T, dependencies: &[T], validate: bool) -> Result<(), DAGError> {
        // Check for self-dependency
        if dependencies.contains(&id) {
            return Err(DAGError::SelfDependency {
                node_id: alloc::format!("{:?}", id),
            });
        }

        // Check if node already exists
        if self.nodes.contains_key(&id) {
            return Err(DAGError::NodeExists {
                node_id: alloc::format!("{:?}", id),
            });
        }

        // Validate dependencies exist (if requested)
        // NOTE: When validation is enabled, cycles are impossible by construction
        // because nodes can only depend on already-existing nodes, ensuring
        // topological ordering during insertion.
        if validate {
            for &dep in dependencies {
                if !self.nodes.contains_key(&dep) {
                    return Err(DAGError::MissingDependency {
                        dep_id: alloc::format!("{:?}", dep),
                    });
                }
            }
        }

        // Calculate rebuild depth: max depth of dependencies + 1
        // Root nodes (no dependencies) have depth 0
        let rebuild_depth = dependencies
            .iter()
            .filter_map(|dep| self.nodes.get(dep))
            .map(|n| n.rebuild_depth)
            .max()
            .map(|d| d.saturating_add(1))
            .unwrap_or(0);

        // Deduplicate dependencies to prevent inflated computation_cost
        // and duplicate entries in parent dependents lists.
        let mut seen = HashSet::with_capacity(dependencies.len());
        let deduped: Vec<T> = dependencies
            .iter()
            .copied()
            .filter(|d| seen.insert(*d))
            .collect();

        let computation_cost = deduped.len();
        let is_root = deduped.is_empty();

        // Add reverse dependencies (use deduped to avoid duplicate entries)
        for &dep in &deduped {
            if let Some(dep_node) = self.nodes.get_mut(&dep) {
                dep_node.dependents.push(id);
            }
        }

        // Update roots: new node is a root if it has no dependencies
        if is_root {
            self.roots.insert(id);
        }

        // Update leaves: dependencies are no longer leaves (they now have dependents)
        for &dep in &deduped {
            self.leaves.remove(&dep);
        }
        // New node is always a leaf until something depends on it
        self.leaves.insert(id);

        let node = DAGNode {
            dependencies: deduped, // move, no clone
            dependents: Vec::new(),
            computation_cost,
            access_frequency: 1,
            creation_time: self.time_counter,
            rebuild_depth,
        };

        self.time_counter = self.time_counter.saturating_add(1);

        self.nodes.insert(id, node);

        // Incremental critical path update: new leaf has depth 0.
        self.critical_paths.insert(id, 0);
        if !dependencies.is_empty() {
            self.propagate_critical_paths(dependencies);
        }

        Ok(())
    }

    /// Maximum dependencies of any single node.
    pub fn max_dependency_width(&self) -> usize {
        self.nodes
            .values()
            .map(|n| n.dependencies.len())
            .max()
            .unwrap_or(0)
    }

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

    #[inline]
    pub fn get_node(&self, id: T) -> Option<&DAGNode<T>> {
        self.nodes.get(&id)
    }

    pub fn mark_accessed(&mut self, id: T) {
        if let Some(node) = self.nodes.get_mut(&id) {
            node.access_frequency = node.access_frequency.saturating_add(1);
        }
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

    pub fn stats(&self) -> DAGStats {
        let max_depth = self.calculate_max_depth();
        let avg_fanout = if !self.nodes.is_empty() {
            self.nodes
                .values()
                .map(|n| n.dependents.len())
                .sum::<usize>() as f64
                / self.nodes.len() as f64
        } else {
            0.0
        };

        DAGStats {
            total_nodes: self.nodes.len(),
            root_nodes: self.roots.len(),
            leaf_nodes: self.leaves.len(),
            max_depth,
            average_fanout: avg_fanout,
        }
    }

    /// Remove a node from the DAG, cleaning up all back-edges.
    ///
    /// Dependencies of this node lose it from their `dependents` list
    /// (and may become leaves). Dependents of this node lose it from
    /// their `dependencies` list (and may become roots).
    pub fn remove_node(&mut self, id: T) -> bool {
        let Some(node) = self.nodes.remove(&id) else {
            return false;
        };

        // Remove from parents' dependents lists.
        for &dep in &node.dependencies {
            if let Some(parent) = self.nodes.get_mut(&dep) {
                parent.dependents.retain(|d| *d != id);
                if parent.dependents.is_empty() {
                    self.leaves.insert(dep);
                }
            }
        }

        // Remove from children's dependencies lists.
        for &child in &node.dependents {
            if let Some(child_node) = self.nodes.get_mut(&child) {
                child_node.dependencies.retain(|d| *d != id);
                if child_node.dependencies.is_empty() {
                    self.roots.insert(child);
                }
            }
        }

        self.roots.remove(&id);
        self.leaves.remove(&id);
        self.critical_paths.remove(&id);

        true
    }

    #[inline]
    pub fn contains(&self, id: T) -> bool {
        self.nodes.contains_key(&id)
    }

    /// Iterate all node IDs in the DAG.
    #[inline]
    pub fn node_ids(&self) -> impl Iterator<Item = &T> {
        self.nodes.keys()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}
