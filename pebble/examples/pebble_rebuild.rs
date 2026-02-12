//! Animated rebuild-from-dependencies example.
//!
//! Same arithmetic pipeline as `pebble_computation`, but each
//! checkpoint records its dependency on the previous checkpoint.
//! When a checkpoint is evicted and later needed, the manager traces
//! back through the DAG and **recomputes** it from its dependencies
//! instead of simply loading serialized bytes from cold storage.
//!
//! This is the core value of pebble: computed data survives eviction
//! because the system knows *how* to recreate it.
//!
//! Run with: cargo run -p pebble --example pebble_rebuild

use pebble::prelude::*;
use std::io::Write;
use std::process::Command;
use std::thread;
use std::time::Duration;

const RED: &str = "\x1b[31m";
const BLUE: &str = "\x1b[34m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";
const HIDE_CURSOR: &str = "\x1b[?25l";
const SHOW_CURSOR: &str = "\x1b[?25h";

const DELAY: Duration = Duration::from_millis(1400);
const HOT_CAPACITY: usize = 3;

// ---------------------------------------------------------------------------
// Checkpoint — each one depends on the previous checkpoint.
//
// The `value` stored is the pipeline result at the moment it was saved.
// `compute_from_dependencies` knows how to get from one checkpoint's
// value to the next by replaying the arithmetic steps between them.
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

    /// Recompute this checkpoint from the previous one.
    ///
    /// Checkpoint layout (values when built correctly):
    ///   #1 = 100   (root — no deps)
    ///   #2 = 300   depends on #1:  100 * 3
    ///   #3 = 488   depends on #2:  (300 - 56) * 2
    ///   #4 = 125   depends on #3:  (488 + 12) / 4
    fn compute_from_dependencies(
        base: Self,
        deps: &HashMap<Self::Id, &Self>,
    ) -> core::result::Result<Self, Self::RebuildError> {
        if base.deps.is_empty() {
            return Ok(base);
        }
        let parent = deps.get(&base.deps[0]).ok_or("missing parent dependency")?;
        let value = match base.id {
            2 => parent.value * 3,
            3 => (parent.value - 56) * 2,
            4 => (parent.value + 12) / 4,
            _ => return Err("unknown checkpoint"),
        };
        Ok(Checkpoint {
            id: base.id,
            value,
            deps: base.deps,
        })
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
// Steps — same Compute / Save / Load / Rebuild pattern as pebble_computation,
// but now saves carry dependency info for the rebuild engine.
// ---------------------------------------------------------------------------

enum Action {
    Compute { step: u64, value: u64 },
    Save { id: u64, dep: Option<u64> },
    Load { id: u64 },
    Rebuild { id: u64 },
}

struct Step {
    label: &'static str,
    action: Action,
}

// ---------------------------------------------------------------------------
// Flow node — tracks computed values for the horizontal chain display.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct FlowNode {
    value: u64,
    cp_id: Option<u64>,
}

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
//   x = 100                      >> save #1
//   x = x * 3 = 300
//   x = x - 56 = 244             >> save #2  (depends on #1)
//   x = x * 2 = 488
//   x = x + 12 = 500             >> save #3  (depends on #2)
//   x = x / 4 = 125
//
// With HOT_CAPACITY = 3, earlier checkpoints get evicted to cold.
// Then we load one from cold and rebuild another from the dep chain.
// ---------------------------------------------------------------------------

fn main() {
    let cold = DirectStorage::new(InMemoryStorage::<u64, u128, 16>::new(), Ser);
    let mut mgr = Mgr::new(cold, NoWarm, Strategy::default(), HOT_CAPACITY);
    let mut val: u64 = 0;
    let mut step_n: u64 = 0;
    let mut flow: Vec<FlowNode> = Vec::new();
    let mut last_action_msg: Option<String> = None;

    #[rustfmt::skip]
    let steps: Vec<Step> = vec![
        // Act 1 — build the pipeline, checkpointing every other result.
        Step { label: "x = 100",                        action: Action::Compute { step: 1, value: 100 } },
        Step { label: "  >> save checkpoint #1",         action: Action::Save { id: 1, dep: None } },
        Step { label: "x = x * 3 = 300",                action: Action::Compute { step: 2, value: 300 } },
        Step { label: "x = x - 56 = 244",               action: Action::Compute { step: 3, value: 244 } },
        Step { label: "  >> save checkpoint #2",         action: Action::Save { id: 2, dep: Some(1) } },
        Step { label: "x = x * 2 = 488",                action: Action::Compute { step: 4, value: 488 } },
        Step { label: "x = x + 12 = 500",               action: Action::Compute { step: 5, value: 500 } },
        Step { label: "  >> save checkpoint #3",         action: Action::Save { id: 3, dep: Some(2) } },
        Step { label: "x = x / 4 = 125",                action: Action::Compute { step: 6, value: 125 } },
        Step { label: "  >> save checkpoint #4",         action: Action::Save { id: 4, dep: Some(3) } },
        // Act 2 — load from cold: data survives serialization round-trip.
        Step { label: "  << load #1 from cold",          action: Action::Load { id: 1 } },
        // Act 3 — rebuild from deps: recompute #3 via #2 via #1.
        Step { label: "  << rebuild #3 from deps",       action: Action::Rebuild { id: 3 } },
    ];

    print!("{HIDE_CURSOR}");
    clear_screen();
    print!(
        "{}",
        render(&mgr, &steps, None, val, step_n, &flow, &last_action_msg)
    );
    print!("  {DIM}Press any key to start...{RESET}");
    std::io::stdout().flush().ok();
    wait_for_key();
    clear_screen();
    print!(
        "{}",
        render(&mgr, &steps, None, val, step_n, &flow, &last_action_msg)
    );
    std::io::stdout().flush().ok();
    thread::sleep(DELAY);

    for i in 0..steps.len() {
        match &steps[i].action {
            Action::Compute { step, value } => {
                val = *value;
                step_n = *step;
                flow.push(FlowNode {
                    value: *value,
                    cp_id: None,
                });
            }
            Action::Save { id, dep } => {
                let deps = match dep {
                    Some(d) => vec![*d],
                    None => vec![],
                };
                mgr.add(Checkpoint {
                    id: *id,
                    value: val,
                    deps,
                })
                .unwrap();
                if let Some(last) = flow.last_mut() {
                    last.cp_id = Some(*id);
                }
                last_action_msg = Some(format!("Saved checkpoint #{id} = {val}"));
            }
            Action::Load { id } => {
                let cp = mgr.load(*id).unwrap();
                let v = cp.value;
                last_action_msg = Some(format!("Loaded #{id} = {v} from cold storage"));
            }
            Action::Rebuild { id } => {
                let cp = mgr.rebuild(*id).unwrap();
                let v = cp.value;
                last_action_msg = Some(format!(
                    "Rebuilt #{id} = {v} by recomputing from dependency chain"
                ));
            }
        }
        clear_screen();
        print!(
            "{}",
            render(&mgr, &steps, Some(i), val, step_n, &flow, &last_action_msg,)
        );
        std::io::stdout().flush().ok();
        thread::sleep(DELAY);
    }

    print!("{SHOW_CURSOR}");
    let s = mgr.stats();
    println!(
        "\n  {BOLD}Done.{RESET} {DIM}Checkpoints: {}  I/O: {}  Utilization: {:.0}%{RESET}\n",
        s.checkpoints_added,
        s.io_operations,
        s.hot_utilization * 100.0,
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
    step_n: u64,
    flow: &[FlowNode],
    action_msg: &Option<String>,
) -> String {
    let mut buf = String::with_capacity(4096);

    // -- Title ---------------------------------------------------------------
    row!(buf, "  {BOLD}Pebble — Rebuild from Dependencies{RESET}");
    row!(buf);

    // -- Step panel ----------------------------------------------------------
    row!(
        buf,
        "  {DIM}┌─────────────────────────────────────────────┐{RESET}"
    );
    for (i, step) in steps.iter().enumerate() {
        match current {
            Some(c) if i < c => {
                let color = match &step.action {
                    Action::Save { .. } => GREEN,
                    Action::Load { .. } => BLUE,
                    Action::Rebuild { .. } => YELLOW,
                    _ => DIM,
                };
                row!(buf, "  {color}  ✓ {}{RESET}", step.label);
            }
            Some(c) if i == c => {
                let color = match &step.action {
                    Action::Save { .. } => GREEN,
                    Action::Load { .. } => BLUE,
                    Action::Rebuild { .. } => YELLOW,
                    _ => BOLD,
                };
                row!(buf, "  {color}{BOLD}  > {}{RESET}", step.label);
            }
            _ => row!(buf, "  {DIM}    {}{RESET}", step.label),
        };
    }
    row!(
        buf,
        "  {DIM}└─────────────────────────────────────────────┘{RESET}"
    );
    row!(buf);

    // -- Current value -------------------------------------------------------
    if step_n > 0 {
        row!(buf, "  {BOLD}x = {val}{RESET}  {DIM}(step {step_n}){RESET}");
    } else {
        row!(buf, "  {DIM}x = ---{RESET}");
    }
    row!(buf);

    // -- Flow chart ----------------------------------------------------------
    if !flow.is_empty() {
        let mut top = String::from("  ");
        let mut bot = String::from("  ");
        for (i, node) in flow.iter().enumerate() {
            let v = format!("{}", node.value);
            let colored = if let Some(id) = node.cp_id {
                if mgr.is_hot(id) {
                    format!("{RED}{BOLD}{v}{RESET}")
                } else {
                    format!("{BLUE}{BOLD}{v}{RESET}")
                }
            } else {
                format!("{DIM}{v}{RESET}")
            };
            top.push_str(&colored);

            let label = if let Some(id) = node.cp_id {
                let l = format!("#{id}");
                if mgr.is_hot(id) {
                    format!("{RED}{l}{RESET}")
                } else {
                    format!("{BLUE}{l}{RESET}")
                }
            } else {
                " ".repeat(v.len()).to_string()
            };
            bot.push_str(&label);

            let lbl_vis = if let Some(id) = node.cp_id {
                format!("#{id}").len()
            } else {
                v.len()
            };
            if lbl_vis < v.len() {
                bot.push_str(&" ".repeat(v.len() - lbl_vis));
            }
            if i + 1 < flow.len() {
                top.push_str(&format!(" {DIM}→{RESET} "));
                bot.push_str("   ");
            }
        }
        row!(buf, "{top}");
        row!(buf, "{bot}");
    }
    row!(buf);

    // -- Tier boxes ----------------------------------------------------------
    let mut hot = Vec::new();
    let mut cold_ids = Vec::new();
    for id in 1..=10u64 {
        if mgr.is_hot(id) {
            hot.push(id);
        } else if mgr.is_in_storage(id) {
            cold_ids.push(id);
        }
    }

    let stats = mgr.stats();
    let used = stats.red_pebble_count;
    let inner: usize = 18;

    let hot_hdr = format!("{RED}{BOLD}HOT{RESET} {DIM}{used}/{HOT_CAPACITY}");
    let hot_hdr_vis = 4 + used.to_string().len() + 1 + HOT_CAPACITY.to_string().len();
    let cold_hdr = format!("{BLUE}{BOLD}COLD{RESET}");
    let cold_hdr_vis: usize = 4;

    let hf = "─".repeat(inner.saturating_sub(hot_hdr_vis + 1));
    let cf = "─".repeat(inner.saturating_sub(cold_hdr_vis + 1));

    row!(
        buf,
        "  ┌─ {hot_hdr} {hf}{RESET}┐    ┌─ {cold_hdr} {DIM}{cf}{RESET}┐"
    );

    let max_rows = HOT_CAPACITY.max(cold_ids.len()).max(1);
    for r in 0..max_rows {
        let (hc, hv) = if r < hot.len() {
            tier_entry(mgr, hot[r], RED)
        } else {
            (String::new(), 0)
        };
        let (cc, cv) = if r < cold_ids.len() {
            tier_entry(mgr, cold_ids[r], BLUE)
        } else {
            (String::new(), 0)
        };
        let hp = inner.saturating_sub(hv);
        let cp = inner.saturating_sub(cv);
        row!(
            buf,
            "  {DIM}│{RESET}  {hc}{:hp$}{DIM}│    │{RESET}  {cc}{:cp$}{DIM}│{RESET}",
            "",
            "",
            hp = hp,
            cp = cp
        );
    }

    let border = "─".repeat(inner + 2);
    row!(buf, "  {DIM}└{border}┘    └{border}┘{RESET}");
    row!(buf);

    // -- Action message ------------------------------------------------------
    if let Some(msg) = action_msg {
        let color = if msg.contains("Rebuilt") {
            YELLOW
        } else if msg.contains("Loaded") {
            BLUE
        } else {
            GREEN
        };
        row!(buf, "  {color}{BOLD}{msg}{RESET}");
        row!(buf);
    }

    // -- Stats ---------------------------------------------------------------
    row!(
        buf,
        "  {DIM}I/O: {}    Checkpoints: {}    Utilization: {:.0}%{RESET}",
        stats.io_operations,
        stats.checkpoints_added,
        stats.hot_utilization * 100.0
    );
    row!(buf);

    buf
}

fn tier_entry(mgr: &Mgr, id: u64, color: &str) -> (String, usize) {
    if let Some(cp) = mgr.get(id) {
        let plain = format!("#{id} = {}", cp.value);
        let vis = plain.len();
        let colored = format!(
            "{color}{BOLD}#{id}{RESET} = {color}{BOLD}{}{RESET}",
            cp.value
        );
        (colored, vis)
    } else {
        let plain = format!("#{id} (stored)");
        let vis = plain.len();
        let colored = format!("{color}{BOLD}#{id}{RESET} {DIM}(stored){RESET}");
        (colored, vis)
    }
}
