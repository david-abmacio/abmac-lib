//! Branch tracking for checkpoint history.
//!
//! Branches are a metadata overlay on the existing `ComputationDAG`.
//! The DAG remains the single source of truth for dependencies;
//! `BranchTracker` adds naming, lineage, and navigation.

use alloc::string::String;
use alloc::vec::Vec;
use hashbrown::HashMap;

/// Auto-assigned branch identifier, separate from checkpoint IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BranchId(pub u64);

/// The default branch, created when branching is enabled.
pub const HEAD: BranchId = BranchId(0);

/// Per-branch metadata.
#[derive(Debug, Clone)]
pub struct BranchInfo<T> {
    /// Branch identifier.
    pub id: BranchId,
    /// Human-readable name.
    pub name: String,
    /// Checkpoint ID where this branch forked off. `None` for HEAD.
    pub fork_point: Option<T>,
    /// Branch this was forked from. `None` for HEAD.
    pub parent: Option<BranchId>,
    /// Most recent checkpoint added to this branch.
    pub head: Option<T>,
}

pub use crate::errors::branch::BranchError;

/// Tracks branch metadata for checkpoints.
///
/// This is a pure metadata structure — it does not own checkpoints
/// or interact with storage. `PebbleManager` owns the tracker and
/// calls into it during `add()`, `remove()`, and `fork()`.
pub(super) struct BranchTracker<T: Copy + Eq + core::hash::Hash> {
    branches: HashMap<BranchId, BranchInfo<T>>,
    checkpoint_to_branch: HashMap<T, BranchId>,
    active: BranchId,
    next_id: u64,
}

impl<T: Copy + Eq + core::hash::Hash> BranchTracker<T> {
    /// Create a new tracker with a HEAD branch.
    pub(super) fn new() -> Self {
        let mut branches = HashMap::new();
        branches.insert(
            HEAD,
            BranchInfo {
                id: HEAD,
                name: String::from("head"),
                fork_point: None,
                parent: None,
                head: None,
            },
        );
        Self {
            branches,
            checkpoint_to_branch: HashMap::new(),
            active: HEAD,
            next_id: 1,
        }
    }

    /// Create a new branch forking from a checkpoint on an existing branch.
    pub(super) fn create_branch(
        &mut self,
        name: &str,
        fork_point: T,
        parent: BranchId,
    ) -> core::result::Result<BranchId, BranchError> {
        if !self.branches.contains_key(&parent) {
            return Err(BranchError::BranchNotFound { id: parent });
        }
        if self.branches.values().any(|b| b.name == name) {
            return Err(BranchError::NameAlreadyUsed {
                name: String::from(name),
            });
        }
        let id = BranchId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        debug_assert!(
            !self.branches.contains_key(&id),
            "branch ID {id:?} collision after wrap-around"
        );
        self.branches.insert(
            id,
            BranchInfo {
                id,
                name: String::from(name),
                fork_point: Some(fork_point),
                parent: Some(parent),
                head: None,
            },
        );
        Ok(id)
    }

    /// Assign a checkpoint to a branch.
    pub(super) fn assign(&mut self, checkpoint_id: T, branch_id: BranchId) {
        self.checkpoint_to_branch.insert(checkpoint_id, branch_id);
    }

    /// Which branch does this checkpoint belong to?
    #[inline]
    pub(super) fn branch_of(&self, checkpoint_id: T) -> Option<BranchId> {
        self.checkpoint_to_branch.get(&checkpoint_id).copied()
    }

    /// Remove a checkpoint from branch tracking.
    pub(super) fn remove_checkpoint(&mut self, checkpoint_id: T) {
        self.checkpoint_to_branch.remove(&checkpoint_id);
    }

    /// Get info for a branch.
    #[inline]
    pub(super) fn info(&self, branch_id: BranchId) -> Option<&BranchInfo<T>> {
        self.branches.get(&branch_id)
    }

    /// Get mutable info for a branch.
    #[inline]
    pub(super) fn info_mut(&mut self, branch_id: BranchId) -> Option<&mut BranchInfo<T>> {
        self.branches.get_mut(&branch_id)
    }

    /// Iterate all branches.
    pub(super) fn all_branches(&self) -> impl Iterator<Item = &BranchInfo<T>> {
        self.branches.values()
    }

    /// Find all branches that fork at a given checkpoint.
    pub(super) fn branches_forked_at(&self, checkpoint_id: T) -> Vec<BranchId> {
        self.branches
            .values()
            .filter(|b| b.fork_point == Some(checkpoint_id))
            .map(|b| b.id)
            .collect()
    }

    /// Walk the parent chain from a branch back to HEAD.
    /// Returns the chain including the starting branch.
    pub(super) fn lineage(&self, branch_id: BranchId) -> Option<Vec<BranchId>> {
        let mut chain = Vec::new();
        let mut current = self.branches.get(&branch_id);
        while let Some(info) = current {
            chain.push(info.id);
            current = info.parent.and_then(|pid| self.branches.get(&pid));
        }
        if chain.is_empty() { None } else { Some(chain) }
    }

    /// The currently active branch.
    #[inline]
    pub(super) fn active(&self) -> BranchId {
        self.active
    }

    /// Set the active branch.
    pub(super) fn set_active(
        &mut self,
        branch_id: BranchId,
    ) -> core::result::Result<(), BranchError> {
        if !self.branches.contains_key(&branch_id) {
            return Err(BranchError::BranchNotFound { id: branch_id });
        }
        self.active = branch_id;
        Ok(())
    }
}
