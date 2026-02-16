//! Eviction logic for PebbleManager.
//!
//! Handles moving checkpoints from the hot tier to the warm/cold tiers
//! when hot capacity is exceeded.

use core::convert::Infallible;

use spout::Spout;

use super::cold::ColdTier;
use super::error::{PebbleManagerError, Result};
use super::manifest::ManifestEntry;
use super::pebble_manager::PebbleManager;
use super::traits::Checkpointable;
use super::warm::WarmTier;

/// Fraction of hot capacity to evict/promote at a time (1/4).
pub(super) const EVICTION_BATCH_DIVISOR: usize = 4;

impl<T, C, W, S> PebbleManager<T, C, W, S>
where
    T: Checkpointable,
    C: ColdTier<T>,
    W: WarmTier<T>,
    S: Spout<ManifestEntry<T::Id>, Error = Infallible>,
{
    pub(super) fn evict_red_pebbles(&mut self) -> Result<usize, T::Id, C::Error> {
        let eviction_count = core::cmp::max(1, self.hot_capacity / EVICTION_BATCH_DIVISOR);

        let candidates =
            self.strategy
                .select_eviction_candidates(&self.red_pebbles, &self.dag, eviction_count);

        let mut evicted = 0;
        for state_id in candidates {
            if let Some(checkpoint) = self.red_pebbles.remove(&state_id) {
                self.evict_single(state_id, checkpoint)?;
                evicted += 1;
            }
        }

        if evicted == 0 && !self.red_pebbles.is_empty() {
            return Err(PebbleManagerError::InternalInconsistency {
                detail: alloc::format!(
                    "eviction produced 0 candidates from {} hot items",
                    self.red_pebbles.len()
                ),
            });
        }

        Ok(evicted)
    }

    pub(super) fn evict_single(
        &mut self,
        state_id: T::Id,
        checkpoint: T,
    ) -> Result<(), T::Id, C::Error> {
        // Remove from the game tracker immediately. The warm tier is not
        // modelled by the pebble game, so the node is temporarily untracked
        // until the warm tier overflows and cold-stores it (place_blue below).
        // This gap is intentional: validate() is only called after the full
        // eviction completes, at which point counts are consistent.
        #[cfg(debug_assertions)]
        self.game.remove_node(state_id);

        if let Some((overflow_id, overflow)) = self.warm.insert(state_id, checkpoint) {
            #[cfg(debug_assertions)]
            self.game.place_blue(overflow_id);

            // Write-ahead: record eviction in manifest BEFORE cold store.
            let deps = self
                .dag
                .get_node(overflow_id)
                .map(|n| n.dependencies())
                .unwrap_or(&[]);
            self.manifest.record(overflow_id, deps)?;

            if let Err(e) = self.cold.store(overflow_id, &overflow) {
                // Cold store failed — undo everything so no data is lost.
                // Remove state_id from warm and put overflow back in its place.
                let recovered = self.warm.remove(state_id);
                self.warm.insert(overflow_id, overflow);

                // Put state_id back into hot (where the caller took it from).
                if let Some(cp) = recovered {
                    self.red_pebbles.insert(state_id, cp);
                }

                #[cfg(debug_assertions)]
                {
                    self.game.remove_node(overflow_id); // undo place_blue
                    if self.red_pebbles.contains_key(&state_id) {
                        self.game.place_red(state_id); // restored to hot
                    }
                }

                return Err(PebbleManagerError::Serialization {
                    state_id: overflow_id,
                    source: e,
                });
            }
            self.blue_pebbles.insert(overflow_id);
            self.dirty.remove(&overflow_id);
            self.io_operations = self.io_operations.saturating_add(1);
        }
        Ok(())
    }
}
