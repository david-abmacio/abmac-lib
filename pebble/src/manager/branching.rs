//! Branching API for PebbleManager.

use alloc::vec::Vec;

use super::branch::{BranchError, BranchId, BranchInfo, BranchTracker, HEAD};
use super::cold::ColdTier;
use super::error::PebbleManagerError;
use super::pebble_manager::PebbleManager;
use super::traits::Checkpointable;
use super::warm::WarmTier;

impl<T, C, W> PebbleManager<T, C, W>
where
    T: Checkpointable,
    C: ColdTier<T>,
    W: WarmTier<T>,
{
    /// Enable branching. Creates a HEAD branch and assigns all
    /// existing checkpoints to it.
    ///
    /// This is a one-time operation. Calling it again is a no-op.
    pub fn enable_branching(&mut self) {
        if self.branches.is_some() {
            return;
        }
        let mut tracker = BranchTracker::new();

        // The DAG contains every checkpoint ID (hot, warm, and cold).
        // A single pass assigns them all to HEAD.
        for &id in self.dag.node_ids() {
            tracker.assign(id, HEAD);
        }

        // Set head to the most recently created checkpoint on HEAD.
        let head = self
            .dag
            .node_ids()
            .max_by_key(|id| {
                self.dag
                    .get_node(**id)
                    .map(|n| n.creation_time)
                    .unwrap_or(0)
            })
            .copied();
        if let Some(info) = tracker.info_mut(HEAD) {
            info.head = head;
        }

        self.branches = Some(tracker);
    }

    /// Create a new branch forking from an existing checkpoint.
    ///
    /// The checkpoint must exist in the manager (hot, warm, or cold tier).
    /// The new branch becomes the active branch. Returns the new branch ID.
    pub fn fork(
        &mut self,
        checkpoint_id: T::Id,
        name: &str,
    ) -> super::error::Result<BranchId, T::Id, C::Error> {
        let tracker =
            self.branches
                .as_mut()
                .ok_or_else(|| PebbleManagerError::InternalInconsistency {
                    detail: alloc::string::String::from("branching not enabled"),
                })?;

        // Verify checkpoint exists.
        if !self.red_pebbles.contains_key(&checkpoint_id)
            && !self.blue_pebbles.contains(&checkpoint_id)
            && !self.warm.contains(checkpoint_id)
        {
            return Err(PebbleManagerError::NeverAdded {
                state_id: checkpoint_id,
            });
        }

        // Determine which branch the fork point belongs to.
        let parent_branch = tracker.branch_of(checkpoint_id).unwrap_or(HEAD);

        let branch_id = tracker
            .create_branch(name, checkpoint_id, parent_branch)
            .map_err(|e| PebbleManagerError::InternalInconsistency {
                detail: alloc::format!("{e}"),
            })?;

        tracker
            .set_active(branch_id)
            .map_err(|e| PebbleManagerError::InternalInconsistency {
                detail: alloc::format!("{e}"),
            })?;

        Ok(branch_id)
    }

    /// Set the active branch. Subsequent `add()` calls assign
    /// checkpoints to this branch.
    pub fn switch_branch(&mut self, branch: BranchId) -> core::result::Result<(), BranchError> {
        let tracker = self
            .branches
            .as_mut()
            .ok_or(BranchError::BranchingNotEnabled)?;
        tracker.set_active(branch)
    }

    /// Return the active branch ID, or `None` if branching is disabled.
    pub fn active_branch(&self) -> Option<BranchId> {
        self.branches.as_ref().map(|t| t.active())
    }

    /// Which branch does this checkpoint belong to?
    /// Returns `None` if branching is disabled or the checkpoint is untracked.
    pub fn branch_of(&self, checkpoint_id: T::Id) -> Option<BranchId> {
        self.branches
            .as_ref()
            .and_then(|t| t.branch_of(checkpoint_id))
    }

    /// List all branches. Returns `None` if branching is disabled.
    pub fn branches(&self) -> Option<Vec<&BranchInfo<T::Id>>> {
        self.branches.as_ref().map(|t| t.all_branches().collect())
    }

    /// Get info for a specific branch.
    pub fn branch_info(&self, branch: BranchId) -> Option<&BranchInfo<T::Id>> {
        self.branches.as_ref().and_then(|t| t.info(branch))
    }

    /// Walk the parent chain from a branch back to HEAD.
    /// Returns `None` if branching is disabled or the branch doesn't exist.
    pub fn branch_lineage(&self, branch: BranchId) -> Option<Vec<BranchId>> {
        self.branches.as_ref().and_then(|t| t.lineage(branch))
    }

    /// Which branches fork at this checkpoint?
    /// Returns `None` if branching is disabled.
    pub fn forks_at(&self, checkpoint_id: T::Id) -> Option<Vec<BranchId>> {
        self.branches
            .as_ref()
            .map(|t| t.branches_forked_at(checkpoint_id))
    }
}
