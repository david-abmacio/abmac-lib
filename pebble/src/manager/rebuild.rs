//! Checkpoint rebuild from dependency graphs.

use alloc::vec::Vec;
use hashbrown::{HashMap, HashSet};

use super::cold::ColdTier;
use super::error::{PebbleManagerError, Result};
use super::pebble_manager::PebbleManager;
use super::safety::CheckpointRef;
use super::traits::Checkpointable;
use super::warm::WarmTier;

impl<T, C, W> PebbleManager<T, C, W>
where
    T: Checkpointable,
    C: ColdTier<T>,
    W: WarmTier<T>,
{
    /// Rebuild a checkpoint from its dependencies.
    ///
    /// Side effects:
    /// - May load multiple checkpoints from storage (one I/O each).
    /// - Clones checkpoints during workspace computation.
    /// - May trigger cascading evictions if hot tier fills during rebuild.
    /// - Marks the target as accessed.
    ///
    /// If the target is already in hot memory with no dependencies,
    /// prefer `get(id).cloned()` instead — it avoids the rebuild overhead.
    #[must_use = "this returns a Result that may indicate an error"]
    pub fn rebuild(&mut self, state_id: T::Id) -> Result<T, T::Id, C::Error> {
        // Warm tier: promote to red_pebbles before rebuild.
        if let Some(checkpoint) = self.warm.get(state_id).cloned() {
            if self.red_pebbles.len() >= self.hot_capacity {
                self.evict_red_pebbles()?;
            }
            self.warm.remove(state_id);
            self.red_pebbles.insert(state_id, checkpoint);

            #[cfg(debug_assertions)]
            self.game.place_red(state_id);
        }

        if let Some(checkpoint) = self.red_pebbles.get(&state_id).cloned() {
            self.dag.mark_accessed(state_id);
            let deps = self
                .dag
                .get_node(state_id)
                .map_or(&[][..], |n| n.dependencies());
            if deps.is_empty() {
                return Ok(checkpoint);
            }
            let dep_refs = self.collect_dependency_refs(deps, state_id)?;
            return Self::compute_from_deps(checkpoint, &dep_refs, state_id);
        }

        // Cold tier: check state exists in storage
        if !self.blue_pebbles.contains(&state_id) {
            return Err(PebbleManagerError::NeverAdded { state_id });
        }

        // Build rebuild plan using DAG
        let available: HashSet<_> = self.red_pebbles.keys().copied().collect();
        let load_order = self.dag.rebuild_order(state_id, &available);

        // Validate: check that max dependency width *in the rebuild plan*
        // doesn't exceed fast memory. Only nodes we actually need to rebuild
        // matter — an unrelated wide node should not block this rebuild.
        let max_width = load_order
            .iter()
            .filter_map(|id| self.dag.get_node(*id))
            .map(|n| n.dependencies.len())
            .max()
            .unwrap_or(0);
        if max_width > self.hot_capacity {
            return Err(PebbleManagerError::DependencyWidthExceeded {
                state_id,
                width: max_width,
                limit: self.hot_capacity,
            });
        }

        // Flush once so all cold items are loadable during the loop.
        self.cold
            .flush()
            .map_err(|e| PebbleManagerError::FlushFailed { source: e })?;

        // Execute rebuild plan with workspace
        let mut workspace: HashMap<T::Id, T> = HashMap::new();

        for node_id in load_order {
            let base_state = self.resolve_base_state(node_id)?;
            let computed = {
                let deps: Vec<T::Id> = self
                    .dag
                    .get_node(node_id)
                    .map(|n| n.dependencies().to_vec())
                    .unwrap_or_default();
                let dep_refs = self.collect_mixed_refs(&deps, &workspace, node_id)?;
                Self::compute_from_deps(base_state, &dep_refs, node_id)?
            };
            workspace.insert(node_id, computed);
        }

        // Extract target from workspace
        let result = workspace
            .remove(&state_id)
            .ok_or(PebbleManagerError::RebuildFailed {
                state_id,
                reason: alloc::string::String::from("target not in workspace after rebuild"),
            })?;

        // Promote valuable workspace items to red pebbles
        self.promote_from_workspace(workspace);

        self.dag.mark_accessed(state_id);
        Ok(result)
    }

    /// Resolve a base state from warm tier (free) or cold storage (I/O).
    ///
    /// Clones from warm rather than removing, so the item stays safe if a
    /// later step in the rebuild loop fails.
    ///
    /// Callers must flush the cold tier before the first call so that
    /// buffered items are visible to `load_from_cold`.
    pub(super) fn resolve_base_state(&mut self, node_id: T::Id) -> Result<T, T::Id, C::Error> {
        if let Some(base) = self.warm.get(node_id).cloned() {
            return Ok(base);
        }

        if !self.blue_pebbles.contains(&node_id) {
            return Err(PebbleManagerError::StorageLoadFailed {
                state_id: node_id,
                reason: alloc::string::String::from("not found in blue_pebbles"),
            });
        }

        self.io_operations = self.io_operations.saturating_add(1);
        self.load_from_cold(node_id)
    }

    /// Load from cold tier, validate size, and return deserialized checkpoint.
    pub(super) fn load_from_cold(&self, state_id: T::Id) -> Result<T, T::Id, C::Error> {
        self.cold
            .load(state_id)
            .map_err(|e| PebbleManagerError::Deserialization {
                state_id,
                source: e,
            })
    }

    fn compute_from_deps(
        base: T,
        dep_refs: &HashMap<T::Id, &T>,
        state_id: T::Id,
    ) -> Result<T, T::Id, C::Error> {
        T::compute_from_dependencies(base, dep_refs).map_err(|e| {
            PebbleManagerError::RebuildFailed {
                state_id,
                reason: alloc::format!("compute_from_dependencies failed: {:?}", e),
            }
        })
    }

    fn collect_dependency_refs(
        &self,
        deps: &[T::Id],
        for_id: T::Id,
    ) -> Result<HashMap<T::Id, &T>, T::Id, C::Error> {
        let mut refs = HashMap::new();
        for &dep_id in deps {
            let dep = self
                .red_pebbles
                .get(&dep_id)
                .ok_or_else(|| PebbleManagerError::DependencyMissing { dep_id, for_id })?;
            refs.insert(dep_id, dep);
        }
        Ok(refs)
    }

    fn collect_mixed_refs<'a>(
        &'a mut self,
        deps: &[T::Id],
        workspace: &'a HashMap<T::Id, T>,
        for_id: T::Id,
    ) -> Result<HashMap<T::Id, &'a T>, T::Id, C::Error> {
        // Deps that were evicted from hot to cold during this rebuild loop
        // need to be loaded back. Collect them first, then borrow immutably.
        let mut cold_loads: Vec<T::Id> = Vec::new();
        for &dep_id in deps {
            if workspace.contains_key(&dep_id)
                || self.red_pebbles.contains_key(&dep_id)
                || self.warm.get(dep_id).is_some()
            {
                continue;
            }
            if self.blue_pebbles.contains(&dep_id) {
                cold_loads.push(dep_id);
            }
        }
        // Promote cold deps back to hot so they're available as refs.
        // This may temporarily push red_pebbles.len() above hot_capacity.
        // The overshoot is bounded by the dependency width of a single node,
        // which rebuild() validated against hot_capacity before entering the
        // rebuild loop. Evicting here would risk removing deps needed by
        // this or subsequent rebuild steps.
        for dep_id in cold_loads {
            let checkpoint = self.load_from_cold(dep_id)?;
            self.io_operations = self.io_operations.saturating_add(1);
            self.red_pebbles.insert(dep_id, checkpoint);
            self.blue_pebbles.remove(&dep_id);

            #[cfg(debug_assertions)]
            self.game.move_to_red(dep_id);
        }

        let mut refs = HashMap::new();
        for &dep_id in deps {
            let dep = workspace
                .get(&dep_id)
                .or_else(|| self.red_pebbles.get(&dep_id))
                .or_else(|| self.warm.get(dep_id))
                .ok_or_else(|| PebbleManagerError::DependencyMissing { dep_id, for_id })?;
            refs.insert(dep_id, dep);
        }
        Ok(refs)
    }

    /// Like [`rebuild`](Self::rebuild), but takes a [`CheckpointRef`] instead of a raw ID.
    #[must_use = "this returns a Result that may indicate an error"]
    pub fn rebuild_ref(&mut self, token: CheckpointRef<T::Id>) -> Result<T, T::Id, C::Error> {
        self.rebuild(token.id())
    }

    pub(super) fn promote_from_workspace(&mut self, mut workspace: HashMap<T::Id, T>) {
        let promote_budget = self.hot_capacity / super::pebble_manager::EVICTION_BATCH_DIVISOR;
        let mut promoted = 0;

        let mut candidates: Vec<_> = workspace.keys().copied().collect();
        candidates.sort_by_key(|id| {
            core::cmp::Reverse(self.dag.get_node(*id).map(|n| n.rebuild_depth).unwrap_or(0))
        });

        for state_id in candidates {
            if promoted >= promote_budget || self.red_pebbles.len() >= self.hot_capacity {
                break;
            }
            if let Some(state) = workspace.remove(&state_id) {
                #[cfg(debug_assertions)]
                let was_blue = self.blue_pebbles.remove(&state_id);
                #[cfg(not(debug_assertions))]
                self.blue_pebbles.remove(&state_id);

                self.red_pebbles.insert(state_id, state);

                #[cfg(debug_assertions)]
                if was_blue {
                    self.game.move_to_red(state_id);
                } else {
                    self.game.place_red(state_id);
                }

                promoted += 1;
            }
        }
    }
}
