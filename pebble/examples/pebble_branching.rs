//! Animated branching example.
//!
//! Builds a linear checkpoint pipeline, then forks from a historical
//! checkpoint to start an alternate computation branch. Shows branch
//! metadata, fork points, and lineage navigation.
//!
//! Run with: cargo run -p pebble --example pebble_branching

use pebble::prelude::*;
use pebble::{BranchId, HEAD};
use std::collections::HashMap as StdHashMap;
use std::io::Write;
use std::process::Command;
use std::thread;
use std::time::Duration;

const RED: &str = "\x1b[31m";
const BLUE: &str = "\x1b[34m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const MAGENTA: &str = "\x1b[35m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";
const HIDE_CURSOR: &str = "\x1b[?25l";
const SHOW_CURSOR: &str = "\x1b[?25h";

const DELAY: Duration = Duration::from_millis(800);
const HOT_CAPACITY: usize = 4;

// ---------------------------------------------------------------------------
// Checkpoint type
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Checkpoint {
    id: u64,
    value: u64,
    deps: Vec<u64>,
}

impl Checkpointable for Checkpoint {
    type Id = u64;
    type RebuildError = &'static str;
    fn checkpoint_id(&self) -> u64 {
        self.id
    }
    fn dependencies(&self) -> &[u64] {
        &self.deps
    }
    fn compute_from_dependencies(
        base: Self,
        _: &HashMap<Self::Id, &Self>,
    ) -> core::result::Result<Self, Self::RebuildError> {
        Ok(base)
    }
}

struct Ser;
impl CheckpointSerializer<Checkpoint> for Ser {
    type Error = &'static str;
    fn serialize(&self, cp: &Checkpoint) -> core::result::Result<Vec<u8>, Self::Error> {
        let mut b = Vec::with_capacity(32);
        b.extend_from_slice(&cp.id.to_be_bytes());
        b.extend_from_slice(&cp.value.to_be_bytes());
        b.extend_from_slice(&(cp.deps.len() as u64).to_be_bytes());
        for d in &cp.deps {
            b.extend_from_slice(&d.to_be_bytes());
        }
        Ok(b)
    }
    fn deserialize(&self, b: &[u8]) -> core::result::Result<Checkpoint, Self::Error> {
        if b.len() < 24 {
            return Err("too small");
        }
        let id = u64::from_be_bytes(b[0..8].try_into().unwrap());
        let val = u64::from_be_bytes(b[8..16].try_into().unwrap());
        let n = u64::from_be_bytes(b[16..24].try_into().unwrap()) as usize;
        let mut deps = Vec::with_capacity(n);
        for i in 0..n {
            deps.push(u64::from_be_bytes(
                b[24 + i * 8..32 + i * 8].try_into().unwrap(),
            ));
        }
        Ok(Checkpoint {
            id,
            value: val,
            deps,
        })
    }
}

type Mgr = PebbleManager<Checkpoint, DirectStorage<InMemoryStorage<u64, u128, 16>, Ser>, NoWarm>;

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Terminal helpers
// ---------------------------------------------------------------------------

fn clear_screen() {
    Command::new("clear").status().ok();
}

fn wait_for_key() {
    let mut tty = std::fs::File::open("/dev/tty").unwrap();
    Command::new("stty")
        .args(["raw", "-echo"])
        .stdin(std::fs::File::open("/dev/tty").unwrap())
        .status()
        .ok();
    use std::io::Read;
    let mut buf = [0u8; 1];
    let _ = tty.read(&mut buf);
    Command::new("stty")
        .args(["cooked", "echo"])
        .stdin(std::fs::File::open("/dev/tty").unwrap())
        .status()
        .ok();
}

macro_rules! row {
    ($b:expr) => {{ $b.push('\n'); }};
    ($b:expr, $($a:tt)*) => {{
        use std::fmt::Write as _;
        writeln!($b, $($a)*).unwrap();
    }};
}

// ---------------------------------------------------------------------------
// Main
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
// ---------------------------------------------------------------------------

fn main() {
    let cold = DirectStorage::new(InMemoryStorage::<u64, u128, 16>::new(), Ser);
    let mut mgr = Mgr::new(cold, NoWarm, Strategy::default(), HOT_CAPACITY);
    let mut val: u64 = 0;
    let mut msg: Option<String> = None;
    let mut values: StdHashMap<u64, u64> = StdHashMap::new();

    // We'll track branch IDs as they're created.
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
                mgr.add(Checkpoint {
                    id: *id,
                    value: val,
                    deps: deps.to_vec(),
                })
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
                msg = Some("Branching enabled — all checkpoints assigned to HEAD".into());
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
                // Use the actual experiment branch ID
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
        s.checkpoints_added,
        s.io_operations,
        mgr.branches().map(|b| b.len()).unwrap_or(0),
    );
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

fn render(
    mgr: &Mgr,
    steps: &[Step],
    current: Option<usize>,
    val: u64,
    action_msg: &Option<String>,
    values: &StdHashMap<u64, u64>,
) -> String {
    let mut buf = String::with_capacity(4096);

    // -- Title ---------------------------------------------------------------
    row!(buf, "  {BOLD}Pebble — Branching{RESET}");
    row!(buf);

    // -- Step panel ----------------------------------------------------------
    row!(
        buf,
        "  {DIM}┌──────────────────────────────────────────────┐{RESET}"
    );
    for (i, step) in steps.iter().enumerate() {
        let color = match &step.action {
            Action::Save { .. } => GREEN,
            Action::Load { .. } => BLUE,
            Action::EnableBranching | Action::Fork { .. } | Action::SwitchBranch { .. } => MAGENTA,
            Action::ShowForks { .. } | Action::ShowLineage { .. } => YELLOW,
            _ => DIM,
        };
        match current {
            Some(c) if i < c => row!(buf, "  {color}  \u{2713} {}{RESET}", step.label),
            Some(c) if i == c => row!(buf, "  {color}{BOLD}  > {}{RESET}", step.label),
            _ => row!(buf, "  {DIM}    {}{RESET}", step.label),
        };
    }
    row!(
        buf,
        "  {DIM}└──────────────────────────────────────────────┘{RESET}"
    );
    row!(buf);

    // -- Current value -------------------------------------------------------
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

    // -- Branch diagram ------------------------------------------------------
    // Shows the DAG as two lines: HEAD and experiment.
    //
    //   HEAD:       #1 --- #2 --- #3 --- #4 --- #7
    //                       |
    //   experiment:         +--- #5 --- #6 --- #8
    if let Some(branches) = mgr.branches() {
        let mut head_ids: Vec<u64> = Vec::new();
        let mut exp_ids: Vec<u64> = Vec::new();
        let mut fork_at: Option<u64> = None;
        let mut exp_branch_name: Option<String> = None;

        // Find the first non-HEAD branch and its fork point from metadata.
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
            // Count characters to align the fork marker under the fork point
            let mut pos = 17; // "  HEAD:       " = 17 visible chars
            for id in &head_ids {
                if *id == fid {
                    pos += format!("#{id}").len() / 2;
                    break;
                }
                pos += format!("#{id}").len() + 5; // " --- "
            }
            let pad = " ".repeat(pos);
            row!(buf, "{pad}{DIM}|{RESET}");

            // Experiment line
            let branch_name = exp_branch_name.clone().unwrap_or_else(|| "?".into());

            let mut exp_line = format!("  {MAGENTA}{BOLD}{branch_name}:{RESET}");
            // Pad to align under the fork point
            let label_len = branch_name.len() + 1; // +1 for ":"
            let needed = 17usize.saturating_sub(2 + label_len);
            exp_line.push_str(&" ".repeat(needed));

            // Offset to fork point
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
            // Reserve space for the fork indicator + experiment line.
            row!(buf);
            row!(buf);
        }
        row!(buf);
    } else {
        // Reserve space for the entire branch diagram (3 lines + blank).
        row!(buf);
        row!(buf);
        row!(buf);
        row!(buf);
    }

    // -- Tier boxes ----------------------------------------------------------
    let mut hot = Vec::new();
    let mut cold_ids = Vec::new();
    for id in 1..=20u64 {
        if mgr.is_hot(id) {
            hot.push(id);
        } else if mgr.is_in_storage(id) {
            cold_ids.push(id);
        }
    }

    let stats = mgr.stats();
    let used = stats.red_pebble_count;
    let inner: usize = 22;

    let hot_hdr = format!("{RED}{BOLD}HOT{RESET} {DIM}{used}/{HOT_CAPACITY}");
    let hot_hdr_vis = 4 + used.to_string().len() + 1 + HOT_CAPACITY.to_string().len();
    let cold_hdr = format!("{BLUE}{BOLD}COLD{RESET}");
    let cold_hdr_vis: usize = 4;

    let hf = "\u{2500}".repeat(inner.saturating_sub(hot_hdr_vis + 1));
    let cf = "\u{2500}".repeat(inner.saturating_sub(cold_hdr_vis + 1));

    row!(
        buf,
        "  \u{250c}\u{2500} {hot_hdr} {hf}{RESET}\u{2510}    \u{250c}\u{2500} {cold_hdr} {DIM}{cf}{RESET}\u{2510}"
    );

    let max_rows = HOT_CAPACITY.max(cold_ids.len()).max(1);
    for r in 0..max_rows {
        let (hc, hv) = if r < hot.len() {
            tier_entry(mgr, hot[r], RED, values)
        } else {
            (String::new(), 0)
        };
        let (cc, cv) = if r < cold_ids.len() {
            tier_entry(mgr, cold_ids[r], BLUE, values)
        } else {
            (String::new(), 0)
        };
        let hp = inner.saturating_sub(hv);
        let cp = inner.saturating_sub(cv);
        row!(
            buf,
            "  {DIM}\u{2502}{RESET}  {hc}{:hp$}{DIM}\u{2502}    \u{2502}{RESET}  {cc}{:cp$}{DIM}\u{2502}{RESET}",
            "",
            "",
            hp = hp,
            cp = cp
        );
    }

    let border = "\u{2500}".repeat(inner + 2);
    row!(
        buf,
        "  {DIM}\u{2514}{border}\u{2518}    \u{2514}{border}\u{2518}{RESET}"
    );
    row!(buf);

    // -- Action message (always reserve the space) ----------------------------
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

    // -- Stats ---------------------------------------------------------------
    row!(
        buf,
        "  {DIM}I/O: {}    Checkpoints: {}    Branches: {}{RESET}",
        stats.io_operations,
        stats.checkpoints_added,
        mgr.branches().map(|b| b.len()).unwrap_or(0),
    );
    row!(buf);

    buf
}

fn tier_entry(mgr: &Mgr, id: u64, color: &str, values: &StdHashMap<u64, u64>) -> (String, usize) {
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
                3 + info.name.len() // " (name)"
            }
        })
        .unwrap_or(0);

    // Get value from hot tier, or fall back to our saved values map.
    let value = mgr
        .get(id)
        .map(|cp| cp.value)
        .or_else(|| values.get(&id).copied());

    if let Some(v) = value {
        let plain_len = format!("#{id} = {v}").len() + tag_vis;
        let colored = format!("{color}{BOLD}#{id}{RESET} = {color}{BOLD}{v}{RESET}{branch_tag}",);
        (colored, plain_len)
    } else {
        let plain_len = format!("#{id}").len() + tag_vis;
        let colored = format!("{color}{BOLD}#{id}{RESET}{branch_tag}");
        (colored, plain_len)
    }
}
