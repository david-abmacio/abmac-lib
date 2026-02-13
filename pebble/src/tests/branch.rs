//! Tests for branching.

use alloc::vec;
use alloc::vec::Vec;

use spout::DropSpout;

use crate::manager::{BranchError, HEAD, Manifest, NoWarm, PebbleManager};
use crate::strategy::Strategy;

use super::fixtures::{TestCheckpoint, cp, test_cold};

#[test]
fn branching_disabled_by_default() {
    let mut mgr = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        10,
    );
    mgr.add(cp(1), &[]).unwrap();
    assert!(mgr.active_branch().is_none());
    assert!(mgr.branch_of(1).is_none());
    assert!(mgr.branches().is_none());
}

#[test]
fn enable_branching_assigns_existing_to_head() {
    let mut mgr = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        10,
    );
    mgr.add(cp(1), &[]).unwrap();
    mgr.add(cp(2), &[]).unwrap();

    mgr.enable_branching();

    assert_eq!(mgr.active_branch(), Some(HEAD));
    assert_eq!(mgr.branch_of(1), Some(HEAD));
    assert_eq!(mgr.branch_of(2), Some(HEAD));

    let info = mgr.branch_info(HEAD).unwrap();
    assert_eq!(info.name, "head");
    assert!(info.fork_point.is_none());
    assert!(info.parent.is_none());
}

#[test]
fn enable_branching_idempotent() {
    let mut mgr = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        10,
    );
    mgr.enable_branching();
    mgr.add(cp(1), &[]).unwrap();
    mgr.enable_branching(); // second call is no-op
    assert_eq!(mgr.branch_of(1), Some(HEAD));
}

#[test]
fn add_assigns_to_active_branch() {
    let mut mgr = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        10,
    );
    mgr.enable_branching();

    mgr.add(cp(1), &[]).unwrap();
    mgr.add(cp(2), &[]).unwrap();

    assert_eq!(mgr.branch_of(1), Some(HEAD));
    assert_eq!(mgr.branch_of(2), Some(HEAD));

    let info = mgr.branch_info(HEAD).unwrap();
    assert_eq!(info.head, Some(2));
}

#[test]
fn fork_creates_branch() {
    let mut mgr = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        10,
    );
    mgr.enable_branching();
    mgr.add(cp(1), &[]).unwrap();
    mgr.add(cp(2), &[1]).unwrap();
    mgr.add(cp(3), &[2]).unwrap();

    let branch = mgr.fork(2, "experiment").unwrap();

    let info = mgr.branch_info(branch).unwrap();
    assert_eq!(info.name, "experiment");
    assert_eq!(info.fork_point, Some(2));
    assert_eq!(info.parent, Some(HEAD));
    assert!(info.head.is_none()); // no checkpoints added to this branch yet
}

#[test]
fn fork_switches_active_branch() {
    let mut mgr = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        10,
    );
    mgr.enable_branching();
    mgr.add(cp(1), &[]).unwrap();

    let branch = mgr.fork(1, "alt").unwrap();
    assert_eq!(mgr.active_branch(), Some(branch));

    // New adds go to the new branch.
    mgr.add(cp(2), &[1]).unwrap();
    assert_eq!(mgr.branch_of(2), Some(branch));
    assert_eq!(mgr.branch_of(1), Some(HEAD)); // unchanged
}

#[test]
fn switch_branch() {
    let mut mgr = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        10,
    );
    mgr.enable_branching();
    mgr.add(cp(1), &[]).unwrap();
    let branch = mgr.fork(1, "alt").unwrap();

    mgr.switch_branch(HEAD).unwrap();
    assert_eq!(mgr.active_branch(), Some(HEAD));

    mgr.switch_branch(branch).unwrap();
    assert_eq!(mgr.active_branch(), Some(branch));
}

#[test]
fn switch_branch_not_found() {
    let mut mgr = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        10,
    );
    mgr.enable_branching();

    use crate::manager::BranchId;
    let result = mgr.switch_branch(BranchId(999));
    assert_eq!(
        result,
        Err(BranchError::BranchNotFound { id: BranchId(999) })
    );
}

#[test]
fn switch_branch_not_enabled() {
    let mut mgr = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        10,
    );
    let result = mgr.switch_branch(HEAD);
    assert_eq!(result, Err(BranchError::BranchingNotEnabled));
}

#[test]
fn fork_nonexistent_checkpoint() {
    let mut mgr = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        10,
    );
    mgr.enable_branching();

    let result = mgr.fork(999, "bad");
    assert!(result.is_err()); // NeverAdded
}

#[test]
fn branch_lineage() {
    let mut mgr = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        10,
    );
    mgr.enable_branching();
    mgr.add(cp(1), &[]).unwrap();
    let b1 = mgr.fork(1, "b1").unwrap();
    mgr.add(cp(2), &[1]).unwrap();
    let b2 = mgr.fork(2, "b2").unwrap();

    let lineage = mgr.branch_lineage(b2).unwrap();
    assert_eq!(lineage, vec![b2, b1, HEAD]);

    let head_lineage = mgr.branch_lineage(HEAD).unwrap();
    assert_eq!(head_lineage, vec![HEAD]);
}

#[test]
fn forks_at() {
    let mut mgr = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        10,
    );
    mgr.enable_branching();
    mgr.add(cp(1), &[]).unwrap();

    let b1 = mgr.fork(1, "b1").unwrap();
    mgr.switch_branch(HEAD).unwrap();
    let b2 = mgr.fork(1, "b2").unwrap();

    let mut forks = mgr.forks_at(1).unwrap();
    forks.sort_by_key(|b| b.0);
    assert_eq!(forks, vec![b1, b2]);

    // No forks at checkpoint 999.
    assert_eq!(mgr.forks_at(999).unwrap(), vec![]);
}

#[test]
fn remove_cleans_branch_tracker() {
    let mut mgr = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        10,
    );
    mgr.enable_branching();
    mgr.add(cp(1), &[]).unwrap();
    assert_eq!(mgr.branch_of(1), Some(HEAD));

    mgr.remove(1);
    assert_eq!(mgr.branch_of(1), None);
}

#[test]
fn branches_lists_all() {
    let mut mgr = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        10,
    );
    mgr.enable_branching();
    mgr.add(cp(1), &[]).unwrap();
    mgr.fork(1, "alt").unwrap();

    let branches = mgr.branches().unwrap();
    assert_eq!(branches.len(), 2);

    let names: Vec<&str> = branches.iter().map(|b| b.name.as_str()).collect();
    assert!(names.contains(&"head"));
    assert!(names.contains(&"alt"));
}

#[test]
fn duplicate_branch_name_rejected() {
    let mut mgr = PebbleManager::<TestCheckpoint, _, _, _>::new(
        test_cold(),
        NoWarm,
        Manifest::new(DropSpout),
        Strategy::default(),
        10,
    );
    mgr.enable_branching();
    mgr.add(cp(1), &[]).unwrap();

    mgr.fork(1, "experiment").unwrap();
    mgr.switch_branch(HEAD).unwrap();
    let result = mgr.fork(1, "experiment");
    assert!(result.is_err()); // NameAlreadyUsed
}
