//! Core checkpoint management implementation.

use alloc::vec::Vec;
use hashbrown::{HashMap, HashSet};

use crate::dag::ComputationDAG;
use crate::strategy::Strategy;

use core::convert::Infallible;

use spout::Spout;

use super::cold::ColdTier;
use super::error::{PebbleManagerError, Result};
use super::manifest::{Manifest, ManifestEntry};
use super::safety::{CapacityGuard, CheckpointRef};
use super::stats::{PebbleStats, TheoreticalValidation};
use super::traits::Checkpointable;
use super::warm::WarmTier;

/// Space bound safety multiplier: hot_capacity <= sqrt(T) * this value.
pub(super) const SPACE_BOUND_MULTIPLIER: usize = 2;

/// Tree strategy I/O approximation bound (Gleinig & Hoefler 2022).
pub(super) const TREE_IO_BOUND: f64 = 2.0;

/// DAG strategy heuristic I/O bound (optimal is PSPACE-hard).
pub(super) const DAG_IO_BOUND: f64 = 3.0;

/// O(sqrt(T)) checkpoint manager using the Red-Blue Pebble Game algorithm.
///
/// # Type Parameters
/// - `T` — Checkpointable type
/// - `C` — Cold tier (serialization + storage)
/// - `W` — Warm tier (unserialized eviction buffer)
/// - `S` — Manifest spout (where WAL entries drain on overflow)
///
/// Not internally synchronized. For concurrent access, wrap in a
/// `Mutex` or `RwLock`. `PebbleManager` is `Send` when all type
/// parameters (`T`, `C`, `W`, `S`) are `Send`.
#[must_use]
pub struct PebbleManager<T, C, W, S>
where
    T: Checkpointable,
    C: ColdTier<T>,
    W: WarmTier<T>,
    S: Spout<ManifestEntry<T::Id>, Error = Infallible>,
{
    pub(super) hot_capacity: usize,
    pub(super) red_pebbles: HashMap<T::Id, T>,
    pub(super) blue_pebbles: HashSet<T::Id>,
    pub(super) dag: ComputationDAG<T::Id>,
    pub(super) strategy: Strategy,
    pub(super) cold: C,
    pub(super) warm: W,
    pub(super) manifest: Manifest<T::Id, S>,
    pub(super) checkpoints_added: u64,
    pub(super) io_operations: u64,
    pub(super) auto_resize: bool,
    /// Hot-tier items that have never been written to cold storage.
    /// Cleared when the item reaches cold (via eviction, flush, or dirty flush).
    pub(super) dirty: HashSet<T::Id>,
    pub(super) branches: Option<super::branch::BranchTracker<T::Id>>,
    #[cfg(debug_assertions)]
    pub(super) game: crate::game::PebbleGame<T::Id>,
}

impl<T, C, W, S> PebbleManager<T, C, W, S>
where
    T: Checkpointable,
    C: ColdTier<T>,
    W: WarmTier<T>,
    S: Spout<ManifestEntry<T::Id>, Error = Infallible>,
{
    /// Create a new PebbleManager.
    pub(crate) fn new(
        cold: C,
        warm: W,
        manifest: Manifest<T::Id, S>,
        strategy: Strategy,
        hot_capacity: usize,
        auto_resize: bool,
    ) -> Self {
        debug_assert!(hot_capacity >= 1, "hot_capacity must be at least 1");
        let hot_capacity = hot_capacity.max(1);
        Self {
            hot_capacity,
            red_pebbles: HashMap::new(),
            blue_pebbles: HashSet::new(),
            dag: ComputationDAG::new(),
            strategy,
            cold,
            warm,
            manifest,
            checkpoints_added: 0,
            io_operations: 0,
            auto_resize,
            dirty: HashSet::new(),
            branches: None,
            #[cfg(debug_assertions)]
            game: crate::game::PebbleGame::new(hot_capacity),
        }
    }

    /// Add a checkpoint. Evicts older checkpoints to storage if fast memory is full.
    ///
    /// Checkpoints are immutable once added. The rebuild engine assumes
    /// stored values are the canonical result of `compute_from_dependencies`,
    /// so mutating a checkpoint after insertion would silently corrupt any
    /// downstream rebuild that depends on it.
    #[must_use = "this returns a Result that may indicate an error"]
    pub fn add(&mut self, checkpoint: T, dependencies: &[T::Id]) -> Result<(), T::Id, C::Error> {
        let state_id = checkpoint.checkpoint_id();

        // Add to DAG (validates dependencies exist)
        self.dag.add_node(state_id, dependencies)?;

        if self.red_pebbles.len() >= self.hot_capacity {
            self.evict_red_pebbles()?;
        }

        self.register_hot(state_id, checkpoint);

        Ok(())
    }

    /// Insert a checkpoint using a deferred constructor. Eviction happens before construction.
    #[must_use = "this returns a Result that may indicate an error"]
    pub fn insert<F>(
        &mut self,
        dependencies: &[T::Id],
        constructor: F,
    ) -> Result<T::Id, T::Id, C::Error>
    where
        F: FnOnce() -> T,
    {
        // Evict first if necessary (before constructing the checkpoint)
        if self.red_pebbles.len() >= self.hot_capacity {
            self.evict_red_pebbles()?;
        }

        // Now construct the checkpoint
        let checkpoint = constructor();
        let state_id = checkpoint.checkpoint_id();

        // Add to DAG (validates dependencies exist)
        self.dag.add_node(state_id, dependencies)?;

        self.register_hot(state_id, checkpoint);

        Ok(state_id)
    }

    /// Like [`add`](Self::add), but also returns a [`CheckpointRef`] token.
    #[must_use = "this returns a Result that may indicate an error"]
    pub fn add_ref(
        &mut self,
        checkpoint: T,
        dependencies: &[T::Id],
    ) -> Result<CheckpointRef<T::Id>, T::Id, C::Error> {
        let state_id = checkpoint.checkpoint_id();
        self.add(checkpoint, dependencies)?;
        Ok(CheckpointRef::new(state_id))
    }

    /// Like [`insert`](Self::insert), but also returns a [`CheckpointRef`] token.
    #[must_use = "this returns a Result that may indicate an error"]
    pub fn insert_ref<F>(
        &mut self,
        dependencies: &[T::Id],
        constructor: F,
    ) -> Result<CheckpointRef<T::Id>, T::Id, C::Error>
    where
        F: FnOnce() -> T,
    {
        let id = self.insert(dependencies, constructor)?;
        Ok(CheckpointRef::new(id))
    }

    /// Probe for a checkpoint across all tiers. Returns a token if found.
    ///
    /// Replaces the pattern `if manager.contains(id) { ... }` with a token
    /// that can flow into [`load_ref`](Self::load_ref) or
    /// [`rebuild_ref`](Self::rebuild_ref).
    pub fn locate(&self, state_id: T::Id) -> Option<CheckpointRef<T::Id>> {
        if self.contains(state_id) {
            Some(CheckpointRef::new(state_id))
        } else {
            None
        }
    }

    /// Like [`load`](Self::load), but takes a [`CheckpointRef`] instead of a raw ID.
    #[must_use = "this returns a Result that may indicate an error"]
    pub fn load_ref(&mut self, token: CheckpointRef<T::Id>) -> Result<&T, T::Id, C::Error> {
        self.load(token.id())
    }

    /// Ensure at least one slot is free in the hot tier.
    ///
    /// Evicts if necessary. Returns a [`CapacityGuard`] proving capacity
    /// exists. The guard borrows the manager mutably, so no other mutation
    /// can invalidate the guarantee before the guard is consumed.
    #[must_use = "this returns a Result that may indicate an error"]
    pub fn ensure_capacity(&mut self) -> Result<CapacityGuard<'_, T, C, W, S>, T::Id, C::Error> {
        if self.red_pebbles.len() >= self.hot_capacity {
            self.evict_red_pebbles()?;
        }
        Ok(CapacityGuard::new(self))
    }

    /// Get the dependency list for a checkpoint. Returns `None` if the checkpoint
    /// is not in the DAG.
    #[inline]
    pub fn dependencies(&self, state_id: T::Id) -> Option<&[T::Id]> {
        self.dag.get_node(state_id).map(|n| n.dependencies())
    }

    /// Get a checkpoint in fast memory. Returns `None` if not in fast memory.
    ///
    /// Checkpoints are immutable once added — mutating a value would
    /// silently invalidate any downstream checkpoint that depends on it
    /// during a rebuild. Use [`remove`](Self::remove) + [`add`](Self::add)
    /// to replace a checkpoint.
    #[inline]
    pub fn get(&self, state_id: T::Id) -> Option<&T> {
        self.red_pebbles.get(&state_id)
    }

    /// Check if a checkpoint is in fast memory (red pebble).
    #[inline]
    pub fn is_hot(&self, state_id: T::Id) -> bool {
        self.red_pebbles.contains_key(&state_id)
    }

    /// Check if a checkpoint is in storage (blue pebble).
    #[inline]
    pub fn is_in_storage(&self, state_id: T::Id) -> bool {
        self.blue_pebbles.contains(&state_id)
    }

    /// Check if a checkpoint is in the warm tier.
    #[inline]
    pub fn is_in_warm(&self, state_id: T::Id) -> bool {
        self.warm.contains(state_id)
    }

    /// Check if a checkpoint exists anywhere.
    #[inline]
    pub fn contains(&self, state_id: T::Id) -> bool {
        self.is_hot(state_id) || self.is_in_storage(state_id) || self.warm.contains(state_id)
    }

    /// Load a checkpoint into fast memory, promoting from any tier.
    ///
    /// Side effects:
    /// - Marks the checkpoint as accessed (affects future eviction ordering).
    /// - May evict other checkpoints from hot tier to make room.
    /// - Moves the checkpoint from warm/cold to hot (tier promotion).
    /// - Counts one I/O operation when loading from cold storage.
    ///
    /// Returns a reference to the checkpoint in fast memory.
    #[must_use = "this returns a Result that may indicate an error"]
    pub fn load(&mut self, state_id: T::Id) -> Result<&T, T::Id, C::Error> {
        // Already hot — just touch and return.
        if self.red_pebbles.contains_key(&state_id) {
            self.dag.mark_accessed(state_id);
            return self.expect_hot(state_id);
        }

        // Warm tier: promote from warm cache (no deserialization).
        if let Some(checkpoint) = self.warm.remove(state_id) {
            if self.red_pebbles.len() >= self.hot_capacity {
                self.evict_red_pebbles()?;
            }
            self.red_pebbles.insert(state_id, checkpoint);
            self.dag.mark_accessed(state_id);

            #[cfg(debug_assertions)]
            self.debug_place_red(state_id);

            return self.expect_hot(state_id);
        }

        // Cold tier: load from storage.
        if !self.blue_pebbles.contains(&state_id) {
            return Err(PebbleManagerError::NeverAdded { state_id });
        }

        self.cold
            .flush()
            .map_err(|e| PebbleManagerError::FlushFailed { source: e })?;

        let checkpoint =
            self.cold
                .load(state_id)
                .map_err(|e| PebbleManagerError::Deserialization {
                    state_id,
                    source: e,
                })?;

        if self.red_pebbles.len() >= self.hot_capacity {
            self.evict_red_pebbles()?;
        }

        self.red_pebbles.insert(state_id, checkpoint);
        self.blue_pebbles.remove(&state_id);
        self.io_operations = self.io_operations.saturating_add(1);
        self.dag.mark_accessed(state_id);

        #[cfg(debug_assertions)]
        {
            self.game.move_to_red(state_id);
            self.debug_validate();
        }

        self.expect_hot(state_id)
    }

    /// Get a reference to a checkpoint that must be in red_pebbles.
    fn expect_hot(&self, state_id: T::Id) -> Result<&T, T::Id, C::Error> {
        self.red_pebbles
            .get(&state_id)
            .ok_or_else(|| PebbleManagerError::InternalInconsistency {
                detail: alloc::format!("load: {:?} not in red_pebbles after insert", state_id),
            })
    }

    /// Resize the hot tier capacity at runtime.
    ///
    /// If `new_capacity` is smaller than the current red pebble count,
    /// excess checkpoints are evicted using the active strategy.
    /// Clamped to a minimum of 1.
    #[must_use = "this returns a Result that may indicate an error"]
    pub fn resize_hot(&mut self, new_capacity: usize) -> Result<(), T::Id, C::Error> {
        let new_capacity = new_capacity.max(1);
        self.hot_capacity = new_capacity;

        #[cfg(debug_assertions)]
        {
            self.game = crate::game::PebbleGame::new(new_capacity);
            for &id in self.red_pebbles.keys() {
                self.game.place_red(id);
            }
            for &id in &self.blue_pebbles {
                self.game.place_blue(id);
            }
        }

        while self.red_pebbles.len() > self.hot_capacity {
            self.evict_red_pebbles()?;
        }

        Ok(())
    }

    /// Resize the hot tier to the theoretically optimal sqrt(T).
    ///
    /// Computes `sqrt(total_checkpoints)` and calls [`resize_hot`](Self::resize_hot).
    /// Useful for long-running systems where T grows over time.
    #[must_use = "this returns a Result that may indicate an error"]
    pub fn resize_optimal(&mut self) -> Result<(), T::Id, C::Error> {
        let total = self.len();
        self.resize_hot(total.isqrt().max(1))
    }

    /// Force eviction of older checkpoints to storage. Returns count evicted.
    #[must_use = "this returns a Result that may indicate an error"]
    pub fn compress(&mut self) -> Result<usize, T::Id, C::Error> {
        let total_count = self.red_pebbles.len() + self.blue_pebbles.len() + self.warm.len();
        let eviction_count = self
            .strategy
            .get_eviction_count(self.red_pebbles.len(), total_count);

        if eviction_count == 0 {
            return Ok(0);
        }

        let candidates =
            self.strategy
                .select_eviction_candidates(&self.red_pebbles, &self.dag, eviction_count);

        let mut compressed = 0;
        for state_id in candidates {
            if let Some(checkpoint) = self.red_pebbles.remove(&state_id) {
                self.evict_single(state_id, checkpoint)?;
                compressed += 1;
            }
        }

        Ok(compressed)
    }

    /// Get current statistics.
    pub fn stats(&self) -> PebbleStats {
        let warm_count = self.warm.len();
        let total_nodes = self.red_pebbles.len() + self.blue_pebbles.len() + warm_count;

        // I/O lower bound: max of three valid bounds.
        let dag_stats = self.dag.stats();
        let s = self.hot_capacity;

        // Counting bound: every node beyond S must be evicted once.
        let counting = total_nodes.saturating_sub(s);

        // Edge bound (Hong & Kung 1981): (|E| - |V|*S) / S.
        let v = dag_stats.total_nodes;
        let e = dag_stats.edge_count;
        let edge = if e > v * s {
            (e - v * s).div_ceil(s)
        } else {
            0
        };

        // Depth bound: critical path exceeding hot capacity.
        let depth = (dag_stats.max_depth + 1).saturating_sub(s);

        let theoretical_min_io = counting.max(edge).max(depth).max(1) as u64;

        // Optimal fast memory is O(sqrt(T))
        let optimal_hot = total_nodes.isqrt();
        let space_complexity_ratio = if optimal_hot > 0 {
            self.hot_capacity as f64 / optimal_hot as f64
        } else {
            1.0
        };

        PebbleStats::new(
            self.checkpoints_added,
            self.red_pebbles.len(),
            self.blue_pebbles.len(),
            warm_count,
            self.cold.buffered_count(),
            self.io_operations,
            if self.hot_capacity > 0 {
                self.red_pebbles.len() as f64 / self.hot_capacity as f64
            } else {
                0.0
            },
            theoretical_min_io,
            if theoretical_min_io > 0 {
                self.io_operations as f64 / theoretical_min_io as f64
            } else {
                1.0
            },
            space_complexity_ratio,
        )
    }

    /// Check if current state meets theoretical bounds.
    pub fn validate_theoretical_bounds(&self) -> TheoreticalValidation {
        let stats = self.stats();
        let total_nodes = self.red_pebbles.len() + self.blue_pebbles.len() + self.warm.len();

        let expected_space = total_nodes.isqrt();
        let space_bound_satisfied = self.hot_capacity <= expected_space * SPACE_BOUND_MULTIPLIER;

        let io_bound_satisfied = match &self.strategy {
            Strategy::Tree(_) => stats.io_optimality_ratio() <= TREE_IO_BOUND,
            Strategy::DAG(_) => stats.io_optimality_ratio() <= DAG_IO_BOUND,
        };

        TheoreticalValidation::new(
            space_bound_satisfied,
            io_bound_satisfied,
            stats.space_complexity_ratio(),
            stats.io_optimality_ratio(),
            expected_space,
            total_nodes,
        )
    }

    /// Flush pending writes to storage.
    pub fn flush(&mut self) -> Result<(), T::Id, C::Error> {
        // Drain warm tier to cold. Collect first so a mid-iteration
        // failure doesn't lose items that haven't been stored yet.
        let mut pending: Vec<(T::Id, T)> = self.warm.drain().collect();
        for i in 0..pending.len() {
            let (id, ref checkpoint) = pending[i];

            if let Err(e) = self.persist_to_cold(id, checkpoint) {
                // Re-insert the remaining (unstored) items back into warm.
                // Safe: warm was fully drained above, so re-inserting the
                // remaining items cannot exceed its capacity.
                for (remaining_id, remaining) in pending.drain(i..) {
                    self.warm.insert(remaining_id, remaining);
                }
                return Err(e);
            }
            self.blue_pebbles.insert(id);
            self.dirty.remove(&id);

            #[cfg(debug_assertions)]
            self.game.place_blue(id);
        }

        // Persist dirty hot-tier items to cold storage. Items stay in
        // red_pebbles for fast access but are now also in cold for
        // durability. We don't add to blue_pebbles to avoid double-counting
        // in stats — recovery uses iter_metadata(), not blue_pebbles.
        let dirty_ids: Vec<T::Id> = self.dirty.drain().collect();
        for id in dirty_ids {
            // Cannot use persist_to_cold here: red_pebbles borrow would
            // conflict with &mut self. Manifest + store are called inline.
            let Some(checkpoint) = self.red_pebbles.get(&id) else {
                continue;
            };
            let deps = self
                .dag
                .get_node(id)
                .map(|n| n.dependencies())
                .unwrap_or(&[]);
            self.manifest.record(id, deps)?;
            self.cold
                .store(id, checkpoint)
                .map_err(|e| PebbleManagerError::Serialization {
                    state_id: id,
                    source: e,
                })?;
            self.io_operations = self.io_operations.saturating_add(1);
        }

        // Flush cold tier buffers to storage
        self.cold
            .flush()
            .map_err(|e| PebbleManagerError::FlushFailed { source: e })?;
        // Flush manifest entries to spout
        self.manifest.flush();
        Ok(())
    }

    /// Flush and finalize. Call this before dropping to observe errors.
    ///
    /// `Drop` calls `flush()` as a safety net but silently discards
    /// errors. Use `close()` when you need to confirm all buffered
    /// data reached storage.
    pub fn close(mut self) -> Result<(), T::Id, C::Error> {
        self.flush()
    }

    /// Number of checkpoints in fast memory (red pebbles).
    #[inline]
    pub fn red_count(&self) -> usize {
        self.red_pebbles.len()
    }

    /// Number of checkpoints in storage (blue pebbles).
    #[inline]
    pub fn blue_count(&self) -> usize {
        self.blue_pebbles.len()
    }

    /// Total number of checkpoints.
    #[inline]
    pub fn len(&self) -> usize {
        self.red_pebbles.len() + self.blue_pebbles.len() + self.warm.len()
    }

    /// Whether the manager has no checkpoints.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.red_pebbles.is_empty() && self.blue_pebbles.is_empty() && self.warm.is_empty()
    }

    /// Borrow the cold tier.
    #[inline]
    pub fn cold(&self) -> &C {
        &self.cold
    }

    /// Mutably borrow the cold tier.
    #[inline]
    pub fn cold_mut(&mut self) -> &mut C {
        &mut self.cold
    }

    /// Borrow the manifest.
    #[inline]
    pub fn manifest(&self) -> &Manifest<T::Id, S> {
        &self.manifest
    }

    /// Remove a checkpoint. Returns `true` if found and removed.
    pub fn remove(&mut self, state_id: T::Id) -> bool {
        // Remove from whichever tier holds it (hot, warm, or cold).
        let was_in_hot = self.red_pebbles.remove(&state_id).is_some();
        let was_in_warm = !was_in_hot && self.warm.remove(state_id).is_some();
        let was_in_cold = !was_in_hot && !was_in_warm && self.blue_pebbles.remove(&state_id);

        if !was_in_hot && !was_in_warm && !was_in_cold {
            return false;
        }

        self.dirty.remove(&state_id);
        self.dag.remove_node(state_id);
        if let Some(ref mut tracker) = self.branches {
            tracker.remove_checkpoint(state_id);
        }

        // Write-ahead: record tombstone first, then delete from cold storage.
        // If we crash between (1) and (2), recovery sees the tombstone and
        // ignores any stale data still in cold storage.
        if was_in_cold {
            self.manifest.record_tombstone(state_id);
            let _ = self.cold.remove(state_id);
        }

        // The game tracks hot and cold tiers (not warm).
        #[cfg(debug_assertions)]
        if was_in_hot || was_in_cold {
            self.game.remove_node(state_id);
            self.debug_validate();
        }

        true
    }

    // --- Internal ---

    /// Write-ahead record + cold store + I/O counter. Returns the cold
    /// store error on failure so callers can handle rollback.
    pub(super) fn persist_to_cold(
        &mut self,
        id: T::Id,
        checkpoint: &T,
    ) -> Result<(), T::Id, C::Error> {
        let deps = self
            .dag
            .get_node(id)
            .map(|n| n.dependencies())
            .unwrap_or(&[]);
        self.manifest.record(id, deps)?;
        self.cold
            .store(id, checkpoint)
            .map_err(|e| PebbleManagerError::Serialization {
                state_id: id,
                source: e,
            })?;
        self.io_operations = self.io_operations.saturating_add(1);
        Ok(())
    }

    /// Insert a checkpoint into the hot tier with all bookkeeping:
    /// red_pebbles, dirty set, counter, branch tracking, debug game, auto-grow.
    pub(super) fn register_hot(&mut self, state_id: T::Id, checkpoint: T) {
        self.red_pebbles.insert(state_id, checkpoint);
        self.dirty.insert(state_id);
        self.checkpoints_added = self.checkpoints_added.saturating_add(1);
        self.track_new_checkpoint(state_id);

        #[cfg(debug_assertions)]
        self.debug_place_red(state_id);

        self.auto_grow();
    }

    /// Assign a checkpoint to the active branch and update branch head.
    pub(super) fn track_new_checkpoint(&mut self, state_id: T::Id) {
        if let Some(ref mut tracker) = self.branches {
            let active = tracker.active();
            tracker.assign(state_id, active);
            if let Some(info) = tracker.info_mut(active) {
                info.head = Some(state_id);
            }
        }
    }

    /// Mirror a red pebble placement in the debug game and validate.
    #[cfg(debug_assertions)]
    pub(super) fn debug_place_red(&mut self, state_id: T::Id) {
        self.game.place_red(state_id);
        self.debug_validate();
    }

    /// If auto_resize is enabled and sqrt(T) has grown past hot_capacity, grow it.
    fn auto_grow(&mut self) {
        if self.auto_resize {
            let new_optimal = self.len().isqrt().max(1);
            if new_optimal > self.hot_capacity {
                self.hot_capacity = new_optimal;
                #[cfg(debug_assertions)]
                {
                    self.game = crate::game::PebbleGame::new(self.hot_capacity);
                    for &id in self.red_pebbles.keys() {
                        self.game.place_red(id);
                    }
                    for &id in &self.blue_pebbles {
                        self.game.place_blue(id);
                    }
                }
            }
        }
    }

    #[cfg(debug_assertions)]
    fn debug_validate(&mut self) {
        self.game.clear_log();
        self.game
            .validate_invariants()
            .expect("pebble game invariant violation");
        debug_assert_eq!(
            self.game.red_count(),
            self.red_pebbles.len(),
            "game red count ({}) != red_pebbles len ({})",
            self.game.red_count(),
            self.red_pebbles.len(),
        );
        debug_assert_eq!(
            self.game.blue_count(),
            self.blue_pebbles.len(),
            "game blue count ({}) != blue_pebbles len ({})",
            self.game.blue_count(),
            self.blue_pebbles.len(),
        );
    }
}

impl<T, C, W, S> Drop for PebbleManager<T, C, W, S>
where
    T: Checkpointable,
    C: ColdTier<T>,
    W: WarmTier<T>,
    S: Spout<ManifestEntry<T::Id>, Error = Infallible>,
{
    fn drop(&mut self) {
        let _ = self.flush();
    }
}
