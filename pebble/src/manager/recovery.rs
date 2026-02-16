//! Recovery from existing storage.

use alloc::vec::Vec;
use hashbrown::{HashMap, HashSet};

use core::convert::Infallible;

use spout::Spout;

use super::cold::RecoverableColdTier;
use super::error::Result;
use super::manifest::{Manifest, ManifestEntry};
use super::pebble_manager::PebbleManager;
use super::traits::Checkpointable;
use super::warm::WarmTier;
use crate::dag::ComputationDAG;
use crate::storage::{IntegrityError, IntegrityErrorKind, RecoveryMode, RecoveryResult};
use crate::strategy::Strategy;

impl<T, C, W, S> PebbleManager<T, C, W, S>
where
    T: Checkpointable,
    C: RecoverableColdTier<T>,
    W: WarmTier<T>,
    S: Spout<ManifestEntry<T::Id>, Error = Infallible>,
{
    /// Recover state from existing storage.
    pub(crate) fn recover(
        cold: C,
        warm: W,
        manifest: Manifest<T::Id, S>,
        strategy: Strategy,
        hot_capacity: usize,
        auto_resize: bool,
    ) -> Result<(Self, RecoveryResult), T::Id, C::Error> {
        let mut manager = Self {
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
            branches: None,
            #[cfg(debug_assertions)]
            game: crate::game::PebbleGame::new(hot_capacity),
        };

        // Collect all checkpoint metadata
        let mut checkpoints: Vec<_> = manager.cold.iter_metadata().collect();

        // Cold start: no existing checkpoints
        if checkpoints.is_empty() {
            return Ok((
                manager,
                RecoveryResult {
                    mode: RecoveryMode::ColdStart,
                    checkpoints_loaded: 0,
                    dag_nodes_rebuilt: 0,
                    latest_state_id: None,
                    integrity_errors: Vec::new(),
                },
            ));
        }

        // Sort by timestamp to ensure dependencies are processed first
        checkpoints.sort_by_key(|(_, meta)| meta.creation_timestamp);

        let mut integrity_errors = Vec::new();
        let mut checkpoints_loaded = 0;
        let mut dag_nodes_rebuilt = 0;
        let mut latest_state_id: Option<T::Id> = None;

        for (state_id, metadata) in checkpoints {
            match manager.recover_checkpoint(state_id, &metadata) {
                Ok(()) => {
                    checkpoints_loaded += 1;
                    dag_nodes_rebuilt += 1;
                    latest_state_id = Some(state_id);
                }
                Err(err) => integrity_errors.push(err),
            }
        }

        // Determine recovery mode
        let mode = if integrity_errors.is_empty() {
            RecoveryMode::WarmRestart
        } else if checkpoints_loaded > 0 {
            RecoveryMode::PartialRecovery
        } else {
            RecoveryMode::ColdStart
        };

        // Promote most recent checkpoints to red pebbles
        manager.promote_recent_to_hot(hot_capacity)?;

        Ok((
            manager,
            RecoveryResult {
                mode,
                checkpoints_loaded,
                dag_nodes_rebuilt,
                latest_state_id: latest_state_id.map(|id| alloc::format!("{:?}", id)),
                integrity_errors,
            },
        ))
    }

    /// Validate and register a single checkpoint during recovery.
    fn recover_checkpoint(
        &mut self,
        state_id: T::Id,
        metadata: &crate::storage::CheckpointMetadata<T::Id>,
    ) -> core::result::Result<(), IntegrityError> {
        let deps = metadata.dependencies();

        // Validate dependencies exist (for non-root checkpoints).
        if let Some(&dep_id) = deps
            .iter()
            .find(|&&dep_id| !self.blue_pebbles.contains(&dep_id) && !self.dag.contains(dep_id))
        {
            return Err(IntegrityError {
                state_id: alloc::format!("{:?}", state_id),
                kind: IntegrityErrorKind::MissingDependency {
                    dep_id: alloc::format!("{:?}", dep_id),
                },
            });
        }

        // Add to DAG.
        if self.dag.add_node(state_id, deps).is_err() {
            return Err(IntegrityError {
                state_id: alloc::format!("{:?}", state_id),
                kind: IntegrityErrorKind::DeserializationFailed,
            });
        }

        self.blue_pebbles.insert(state_id);
        #[cfg(debug_assertions)]
        self.game.place_blue(state_id);

        Ok(())
    }

    fn promote_recent_to_hot(&mut self, count: usize) -> Result<(), T::Id, C::Error> {
        if self.blue_pebbles.is_empty() || count == 0 {
            return Ok(());
        }

        let mut candidates: Vec<_> = self.blue_pebbles.iter().copied().collect();
        candidates.sort_by_key(|id| {
            core::cmp::Reverse(self.dag.get_node(*id).map_or(0, |n| n.creation_time))
        });
        candidates.truncate(count);

        for state_id in candidates {
            if self.red_pebbles.len() >= self.hot_capacity {
                break;
            }

            // Best-effort: skip failures during recovery promotion
            let Ok(checkpoint) = self.load_from_cold(state_id) else {
                continue;
            };

            self.red_pebbles.insert(state_id, checkpoint);
            self.blue_pebbles.remove(&state_id);
            self.io_operations = self.io_operations.saturating_add(1);

            #[cfg(debug_assertions)]
            self.game.move_to_red(state_id);
        }

        Ok(())
    }
}
