//! Animated branching example.
//!
//! Builds a linear checkpoint pipeline, then forks from a historical
//! checkpoint to start an alternate computation branch. Shows branch
//! metadata, fork points, and lineage navigation.
//!
//! Run with: cargo run -p pebble --example pebble_branching

#[allow(unused)]
mod common;

use common::*;
use pebble::{BranchId, HEAD};
use std::collections::HashMap as StdHashMap;
use std::io::Write;
use std::thread;
use std::time::Duration;

const DELAY: Duration = Duration::from_millis(800);
const HOT_CAPACITY: usize = 4;

// ── Actions ─────────────────────────────────────────────────────────────────

enum Action {
    Compute { value: u64 },
    Save { id: u64, deps: &'static [u64] },
    Load { id: u64 },
    EnableBranching,
    Fork { from_id: u64, name: &'static str },
    SwitchBranch { branch: BranchId },
    ShowLineage { branch: BranchId },
    ShowForks { checkpoint_id: u64 },
}

struct Step {
    label: &'static str,
    action: Action,
}

// ── Main ────────────────────────────────────────────────────────────────────
//
// Pipeline:
//   #1 = 100  #2 = 300  #3 = 488  #4 = 500
//
// Enable branching, fork from #2 to start "experiment":
//   #5 = 600  #6 = 1200  (on experiment)
//
// Switch back to HEAD and continue:
//   #7 = 750  (on HEAD)
//
// Switch to experiment, load #6 from cold, continue:
//   #8 = 1250  (on experiment)
//
// Switch to HEAD, load #7 — hot now holds both branches.
// Show lineage, fork points.

fn main() {
    let mut mgr = new_manager(HOT_CAPACITY);
    let mut val: u64 = 0;
    let mut msg: Option<String> = None;
    let mut values: StdHashMap<u64, u64> = StdHashMap::new();
    let mut experiment_branch = BranchId(0); // placeholder

    #[rustfmt::skip]
    let steps: Vec<Step> = vec![
        // Act 1 — build a linear pipeline on HEAD.
        Step { label: "x = 100",                           action: Action::Compute { value: 100 } },
        Step { label: "  >> save #1",                      action: Action::Save { id: 1, deps: &[] } },
        Step { label: "x = x * 3 = 300",                  action: Action::Compute { value: 300 } },
        Step { label: "  >> save #2",                      action: Action::Save { id: 2, deps: &[1] } },
        Step { label: "x = x + 188 = 488",                action: Action::Compute { value: 488 } },
        Step { label: "  >> save #3",                      action: Action::Save { id: 3, deps: &[2] } },
        Step { label: "x = x + 12 = 500",                 action: Action::Compute { value: 500 } },
        Step { label: "  >> save #4",                      action: Action::Save { id: 4, deps: &[3] } },

        // Act 2 — enable branching and fork from #2.
        Step { label: "  ** enable branching",             action: Action::EnableBranching },
        Step { label: "  ** fork \"experiment\" from #2",  action: Action::Fork { from_id: 2, name: "experiment" } },

        // Act 3 — add checkpoints on the experiment branch (forked from #2).
        Step { label: "x = 300 * 2 = 600",                action: Action::Compute { value: 600 } },
        Step { label: "  >> save #5 (experiment)",         action: Action::Save { id: 5, deps: &[2] } },
        Step { label: "x = 600 * 2 = 1200",               action: Action::Compute { value: 1200 } },
        Step { label: "  >> save #6 (experiment)",         action: Action::Save { id: 6, deps: &[5] } },

        // Act 4 — switch back to HEAD and continue.
        Step { label: "  ** switch to HEAD",               action: Action::SwitchBranch { branch: HEAD } },
        Step { label: "x = 500 + 250 = 750",              action: Action::Compute { value: 750 } },
        Step { label: "  >> save #7 (HEAD)",               action: Action::Save { id: 7, deps: &[4] } },

        // Act 5 — switch back to experiment, load #6, and continue.
        Step { label: "  ** switch to experiment",         action: Action::SwitchBranch { branch: BranchId(1) } },
        Step { label: "  << load #6 (cold -> hot)",        action: Action::Load { id: 6 } },
        Step { label: "x = 1200 + 50 = 1250",             action: Action::Compute { value: 1250 } },
        Step { label: "  >> save #8 (experiment)",         action: Action::Save { id: 8, deps: &[6] } },

        // Act 6 — switch to HEAD and load #7 so hot shows both branches.
        Step { label: "  ** switch to HEAD",               action: Action::SwitchBranch { branch: HEAD } },
        Step { label: "  << load #7 (cold -> hot)",        action: Action::Load { id: 7 } },

        // Act 7 — inspect branch metadata.
        Step { label: "  ?? forks at #2",                  action: Action::ShowForks { checkpoint_id: 2 } },
        Step { label: "  ?? lineage of experiment",        action: Action::ShowLineage { branch: BranchId(1) } },
    ];

    print!("{HIDE_CURSOR}");
    clear_screen();
    print!("{}", render(&mgr, &steps, None, val, &msg, &values));
    print!("  {DIM}Press any key to start...{RESET}");
    std::io::stdout().flush().ok();
    wait_for_key();
    clear_screen();
    print!("{}", render(&mgr, &steps, None, val, &msg, &values));
    std::io::stdout().flush().ok();
    thread::sleep(DELAY);

    for i in 0..steps.len() {
        match &steps[i].action {
            Action::Compute { value } => {
                val = *value;
                msg = None;
            }
            Action::Load { id } => {
                mgr.load(*id).unwrap();
                let loaded_val = mgr.get(*id).map(|cp| cp.value).unwrap_or(0);
                val = loaded_val;
                msg = Some(format!("Loaded #{id} = {loaded_val} from cold to hot"));
            }
            Action::Save { id, deps } => {
                mgr.add(
                    Checkpoint {
                        id: *id,
                        value: val,
                    },
                    deps,
                )
                .unwrap();
                values.insert(*id, val);
                let branch_name = mgr
                    .active_branch()
                    .and_then(|b| mgr.branch_info(b))
                    .map(|info| info.name.as_str())
                    .unwrap_or("(no branch)");
                msg = Some(format!("Saved #{id} = {val} on {branch_name}"));
            }
            Action::EnableBranching => {
                mgr.enable_branching();
                msg = Some("Branching enabled \u{2014} all checkpoints assigned to HEAD".into());
            }
            Action::Fork { from_id, name } => {
                let bid = mgr.fork(*from_id, name).unwrap();
                experiment_branch = bid;
                msg = Some(format!(
                    "Forked \"{name}\" (branch {}) from checkpoint #{from_id}",
                    bid.0
                ));
            }
            Action::SwitchBranch { branch } => {
                let bid = if *branch == BranchId(1) {
                    experiment_branch
                } else {
                    *branch
                };
                mgr.switch_branch(bid).unwrap();
                let name = mgr.branch_info(bid).map(|b| b.name.as_str()).unwrap();
                msg = Some(format!("Switched to branch \"{name}\""));
            }
            Action::ShowForks { checkpoint_id } => {
                let forks = mgr.forks_at(*checkpoint_id).unwrap_or_default();
                let names: Vec<&str> = forks
                    .iter()
                    .filter_map(|b| mgr.branch_info(*b).map(|info| info.name.as_str()))
                    .collect();
                msg = Some(format!("Forks at #{checkpoint_id}: {names:?}"));
            }
            Action::ShowLineage { branch } => {
                let bid = if *branch == BranchId(1) {
                    experiment_branch
                } else {
                    *branch
                };
                let lineage = mgr.branch_lineage(bid).unwrap_or_default();
                let names: Vec<&str> = lineage
                    .iter()
                    .filter_map(|b| mgr.branch_info(*b).map(|info| info.name.as_str()))
                    .collect();
                msg = Some(format!("Lineage: {}", names.join(" -> ")));
            }
        }
        clear_screen();
        print!("{}", render(&mgr, &steps, Some(i), val, &msg, &values));
        std::io::stdout().flush().ok();
        thread::sleep(DELAY);
    }

    print!("{SHOW_CURSOR}");
    let s = mgr.stats();
    println!(
        "\n  {BOLD}Done.{RESET} {DIM}Checkpoints: {}  I/O: {}  Branches: {}{RESET}\n",
        s.checkpoints_added(),
        s.io_operations(),
        mgr.branches().map(|b| b.len()).unwrap_or(0),
    );
}

// ── Render ──────────────────────────────────────────────────────────────────

fn render(
    mgr: &Mgr,
    steps: &[Step],
    current: Option<usize>,
    val: u64,
    action_msg: &Option<String>,
    values: &StdHashMap<u64, u64>,
) -> String {
    let mut buf = String::with_capacity(4096);

    // Title
    row!(buf, "  {BOLD}Pebble \u{2014} Branching{RESET}");
    row!(buf);

    // Step panel
    let labels: Vec<&str> = steps.iter().map(|s| s.label).collect();
    render_step_panel(&mut buf, &labels, current, |i| match &steps[i].action {
        Action::Save { .. } => GREEN,
        Action::Load { .. } => BLUE,
        Action::EnableBranching | Action::Fork { .. } | Action::SwitchBranch { .. } => MAGENTA,
        Action::ShowForks { .. } | Action::ShowLineage { .. } => YELLOW,
        _ => DIM,
    });

    // Current value
    if val > 0 {
        let branch_label = mgr
            .active_branch()
            .and_then(|b| mgr.branch_info(b))
            .map(|info| info.name.clone())
            .unwrap_or_default();
        if branch_label.is_empty() {
            row!(buf, "  {BOLD}x = {val}{RESET}");
        } else {
            row!(
                buf,
                "  {BOLD}x = {val}{RESET}  {DIM}(on {branch_label}){RESET}"
            );
        }
    } else {
        row!(buf, "  {DIM}x = ---{RESET}");
    }
    row!(buf);

    // Branch diagram
    if let Some(branches) = mgr.branches() {
        let mut head_ids: Vec<u64> = Vec::new();
        let mut exp_ids: Vec<u64> = Vec::new();
        let mut fork_at: Option<u64> = None;
        let mut exp_branch_name: Option<String> = None;

        for b in &branches {
            if b.id != HEAD {
                fork_at = b.fork_point;
                exp_branch_name = Some(b.name.clone());
                break;
            }
        }

        for id in 1..=20u64 {
            if let Some(bid) = mgr.branch_of(id) {
                if bid == HEAD {
                    head_ids.push(id);
                } else {
                    exp_ids.push(id);
                }
            }
        }

        // HEAD line
        let mut head_line = format!("  {BOLD}HEAD:{RESET}       ");
        for (i, id) in head_ids.iter().enumerate() {
            let color = if mgr.is_hot(*id) { RED } else { BLUE };
            head_line.push_str(&format!("{color}{BOLD}#{id}{RESET}"));
            if i + 1 < head_ids.len() {
                head_line.push_str(&format!(" {DIM}---{RESET} "));
            }
        }
        row!(buf, "{head_line}");

        // Fork indicator
        if let Some(fid) = fork_at {
            let mut pos = 17;
            for id in &head_ids {
                if *id == fid {
                    pos += format!("#{id}").len() / 2;
                    break;
                }
                pos += format!("#{id}").len() + 5;
            }
            let pad = " ".repeat(pos);
            row!(buf, "{pad}{DIM}|{RESET}");

            let branch_name = exp_branch_name.clone().unwrap_or_else(|| "?".into());
            let mut exp_line = format!("  {MAGENTA}{BOLD}{branch_name}:{RESET}");
            let label_len = branch_name.len() + 1;
            let needed = 17usize.saturating_sub(2 + label_len);
            exp_line.push_str(&" ".repeat(needed));

            let mut fpos = 0;
            for id in &head_ids {
                if *id == fid {
                    break;
                }
                fpos += format!("#{id}").len() + 5;
            }
            exp_line.push_str(&" ".repeat(fpos));
            exp_line.push_str(&format!("{DIM}+---{RESET} "));

            for (i, id) in exp_ids.iter().enumerate() {
                let color = if mgr.is_hot(*id) { RED } else { BLUE };
                exp_line.push_str(&format!("{color}{BOLD}#{id}{RESET}"));
                if i + 1 < exp_ids.len() {
                    exp_line.push_str(&format!(" {DIM}---{RESET} "));
                }
            }
            row!(buf, "{exp_line}");
        } else {
            row!(buf);
            row!(buf);
        }
        row!(buf);
    } else {
        row!(buf);
        row!(buf);
        row!(buf);
        row!(buf);
    }

    // Tier boxes (with branch-aware entries)
    let (hot, cold) = collect_tiers(mgr, 20);
    let stats = mgr.stats();
    render_tier_boxes(
        &mut buf,
        &hot,
        &cold,
        stats.red_pebble_count(),
        HOT_CAPACITY,
        |id, color| branching_tier_entry(mgr, id, color, values),
    );
    row!(buf);

    // Action message
    if let Some(m) = action_msg {
        let color = if m.contains("Fork") || m.contains("Switch") || m.contains("enabled") {
            MAGENTA
        } else if m.contains("Lineage") || m.contains("Forks") {
            YELLOW
        } else if m.contains("Loaded") {
            BLUE
        } else {
            GREEN
        };
        row!(buf, "  {color}{BOLD}{m}{RESET}");
    } else {
        row!(buf);
    }
    row!(buf);

    // Stats
    row!(
        buf,
        "  {DIM}I/O: {}    Checkpoints: {}    Branches: {}{RESET}",
        stats.io_operations(),
        stats.checkpoints_added(),
        mgr.branches().map(|b| b.len()).unwrap_or(0),
    );
    row!(buf);

    buf
}

/// Tier entry that includes branch name tags.
fn branching_tier_entry(
    mgr: &Mgr,
    id: u64,
    color: &str,
    values: &StdHashMap<u64, u64>,
) -> (String, usize) {
    let branch_tag = mgr
        .branch_of(id)
        .and_then(|b| mgr.branch_info(b))
        .map(|info| {
            if info.id == HEAD {
                String::new()
            } else {
                format!(" {DIM}({}){RESET}", info.name)
            }
        })
        .unwrap_or_default();
    let tag_vis = mgr
        .branch_of(id)
        .and_then(|b| mgr.branch_info(b))
        .map(|info| {
            if info.id == HEAD {
                0
            } else {
                3 + info.name.len()
            }
        })
        .unwrap_or(0);

    let value = mgr
        .get(id)
        .map(|cp| cp.value)
        .or_else(|| values.get(&id).copied());

    if let Some(v) = value {
        let plain_len = format!("#{id} = {v}").len() + tag_vis;
        let colored = format!("{color}{BOLD}#{id}{RESET} = {color}{BOLD}{v}{RESET}{branch_tag}");
        (colored, plain_len)
    } else {
        let plain_len = format!("#{id}").len() + tag_vis;
        let colored = format!("{color}{BOLD}#{id}{RESET}{branch_tag}");
        (colored, plain_len)
    }
}
