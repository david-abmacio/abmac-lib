//! Animated computation pipeline with selective checkpointing.
//!
//! A pipeline runs many computation steps, but only some are
//! checkpointed. The manager keeps recent checkpoints in hot memory
//! and evicts older ones to cold storage. When a rollback happens,
//! the manager loads the nearest checkpoint from cold — anything
//! between checkpoints must be recomputed.
//!
//! Run with: cargo run -p pebble --example pebble_computation

use pebble::prelude::*;
use std::io::Write;
use std::process::Command;
use std::thread;
use std::time::Duration;

const RED: &str = "\x1b[31m";
const BLUE: &str = "\x1b[34m";
const GREEN: &str = "\x1b[32m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";
const HIDE_CURSOR: &str = "\x1b[?25l";
const SHOW_CURSOR: &str = "\x1b[?25h";

const DELAY: Duration = Duration::from_millis(1200);
const HOT_CAPACITY: usize = 3;

// ---------------------------------------------------------------------------
// Checkpoint type — just stores an id and the value at that point.
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
// Animation types
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct FlowNode {
    value: u64,
    cp_id: Option<u64>,
}

enum Action {
    Compute { step: u64, value: u64 },
    Save { id: u64 },
    Rollback { id: u64 },
}

struct Step {
    label: &'static str,
    action: Action,
}

fn clear_screen() {
    Command::new("clear").status().ok();
}

fn wait_for_key() {
    // Set terminal to raw mode, read one byte, restore.
    let mut tty = std::fs::File::open("/dev/tty").unwrap();
    // stty raw -echo
    Command::new("stty")
        .args(["raw", "-echo"])
        .stdin(std::fs::File::open("/dev/tty").unwrap())
        .status()
        .ok();
    use std::io::Read;
    let mut buf = [0u8; 1];
    let _ = tty.read(&mut buf);
    // stty cooked echo
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
// ---------------------------------------------------------------------------

fn main() {
    let cold = DirectStorage::new(InMemoryStorage::<u64, u128, 16>::new(), Ser);
    let mut mgr = Mgr::new(cold, NoWarm, Strategy::default(), HOT_CAPACITY);
    let mut val: u64 = 0;
    let mut step_n: u64 = 0;
    let mut flow: Vec<FlowNode> = Vec::new();
    let mut restored = false;

    #[rustfmt::skip]
    let steps: Vec<Step> = vec![
        Step { label: "x = 100",            action: Action::Compute { step: 1, value: 100 } },
        Step { label: "x = x * 3 = 300",    action: Action::Compute { step: 2, value: 300 } },
        Step { label: "  >> save checkpoint #1",   action: Action::Save { id: 1 } },
        Step { label: "x = x - 56 = 244",   action: Action::Compute { step: 3, value: 244 } },
        Step { label: "x = x * 2 = 488",    action: Action::Compute { step: 4, value: 488 } },
        Step { label: "  >> save checkpoint #2",   action: Action::Save { id: 2 } },
        Step { label: "x = x + 12 = 500",   action: Action::Compute { step: 5, value: 500 } },
        Step { label: "x = x / 4 = 125",    action: Action::Compute { step: 6, value: 125 } },
        Step { label: "  >> save checkpoint #3",   action: Action::Save { id: 3 } },
        Step { label: "x = x + 75 = 200",   action: Action::Compute { step: 7, value: 200 } },
        Step { label: "x = x * 5 = 1000",   action: Action::Compute { step: 8, value: 1000 } },
        Step { label: "  >> save checkpoint #4",   action: Action::Save { id: 4 } },
        Step { label: "  << rollback to #2 (488)", action: Action::Rollback { id: 2 } },
        Step { label: "x = x - 8 = 480",    action: Action::Compute { step: 9, value: 480 } },
        Step { label: "  >> save checkpoint #5",   action: Action::Save { id: 5 } },
    ];

    print!("{HIDE_CURSOR}");
    clear_screen();
    print!(
        "{}",
        render(&mgr, &steps, None, val, step_n, restored, &flow)
    );
    print!("  {DIM}Press any key to start...{RESET}");
    std::io::stdout().flush().ok();
    wait_for_key();
    clear_screen();
    print!(
        "{}",
        render(&mgr, &steps, None, val, step_n, restored, &flow)
    );
    std::io::stdout().flush().ok();
    thread::sleep(DELAY);

    for i in 0..steps.len() {
        match &steps[i].action {
            Action::Compute { step, value } => {
                val = *value;
                step_n = *step;
                restored = false;
                flow.push(FlowNode {
                    value: *value,
                    cp_id: None,
                });
            }
            Action::Save { id } => {
                mgr.add(Checkpoint {
                    id: *id,
                    value: val,
                    deps: vec![],
                })
                .unwrap();
                if let Some(last) = flow.last_mut() {
                    last.cp_id = Some(*id);
                }
            }
            Action::Rollback { id } => {
                let cp = mgr.load(*id).unwrap();
                val = cp.value;
                step_n = 0;
                restored = true;
            }
        }
        clear_screen();
        print!(
            "{}",
            render(&mgr, &steps, Some(i), val, step_n, restored, &flow)
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
    restored: bool,
    flow: &[FlowNode],
) -> String {
    let mut buf = String::with_capacity(4096);

    // -- Step panel ----------------------------------------------------------
    row!(buf, "  {DIM}┌─────────────────────────────────────┐{RESET}");
    for (i, step) in steps.iter().enumerate() {
        match current {
            Some(c) if i < c => {
                let color = match &step.action {
                    Action::Save { .. } => GREEN,
                    Action::Rollback { .. } => BLUE,
                    _ => DIM,
                };
                row!(buf, "  {color}  ✓ {}{RESET}", step.label);
            }
            Some(c) if i == c => row!(buf, "  {BOLD}  > {}{RESET}", step.label),
            _ => row!(buf, "  {DIM}    {}{RESET}", step.label),
        };
    }
    row!(buf, "  {DIM}└─────────────────────────────────────┘{RESET}");
    row!(buf);

    // -- Current value -------------------------------------------------------
    if restored {
        row!(
            buf,
            "  {GREEN}{BOLD}x = {val}{RESET}  {DIM}(restored from cold){RESET}"
        );
    } else if step_n > 0 {
        row!(buf, "  {BOLD}x = {val}{RESET}  {DIM}(step {step_n}){RESET}");
    } else {
        row!(buf, "  {DIM}x = ---{RESET}");
    }
    row!(buf);

    // -- Flow chart ----------------------------------------------------------
    // Shows each computed value; checkpointed ones are colored by tier.
    //   100 → 300 → 244 → 488 → 500 → 125 → 200 → 1000
    //          #1          #2          #3            #4
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
            // Bottom row: checkpoint label or spaces.
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
            // Pad label to match value width.
            let lbl_vis = if node.cp_id.is_some() {
                format!("#{}", node.cp_id.unwrap()).len()
            } else {
                v.len()
            };
            if lbl_vis < v.len() {
                bot.push_str(&" ".repeat(v.len() - lbl_vis));
            }
            if i + 1 < flow.len() {
                top.push_str(&format!(" {DIM}→{RESET} "));
                bot.push_str("   "); // spacing under arrow
            }
        }
        row!(buf, "{top}");
        row!(buf, "{bot}");
    }
    row!(buf);

    // -- Tier boxes ----------------------------------------------------------
    let mut hot = Vec::new();
    let mut cold = Vec::new();
    for id in 1..=20u64 {
        if mgr.is_hot(id) {
            hot.push(id);
        } else if mgr.is_in_storage(id) {
            cold.push(id);
        }
    }

    let stats = mgr.stats();
    let used = stats.red_pebble_count;
    let inner: usize = 18;

    // Header visible chars: "HOT 3/3" = 4 + used_digits + 1 + cap_digits
    let hot_hdr = format!("{RED}{BOLD}HOT{RESET} {DIM}{used}/{HOT_CAPACITY}");
    let hot_hdr_vis = 4 + used.to_string().len() + 1 + HOT_CAPACITY.to_string().len();
    let cold_hdr = format!("{BLUE}{BOLD}COLD{RESET}");
    let cold_hdr_vis: usize = 4;
    // Top: ┌─ {hdr} {fill}┐  must equal  └{inner+2 dashes}┘ in width.
    // Total = 1 + 2 + hdr_vis + 1 + fill + 1 = inner + 4
    // fill = inner - hdr_vis
    // But inner+4 = 22, and 1+2+7+1+fill+1 = 12+fill. 12+fill=22 => fill=10.
    // inner - hdr_vis = 18 - 7 = 11. Off by one because of the space after "─ ".
    // Correct: fill = inner - hdr_vis - 1
    let hf = "─".repeat(inner.saturating_sub(hot_hdr_vis + 1));
    let cf = "─".repeat(inner.saturating_sub(cold_hdr_vis + 1));

    row!(
        buf,
        "  ┌─ {hot_hdr} {hf}{RESET}┐    ┌─ {cold_hdr} {DIM}{cf}{RESET}┐"
    );

    let max_rows = HOT_CAPACITY.max(cold.len()).max(1);
    for r in 0..max_rows {
        let (hc, hv) = if r < hot.len() {
            cp_row(mgr, hot[r], RED)
        } else {
            (String::new(), 0)
        };
        let (cc, cv) = if r < cold.len() {
            cp_row(mgr, cold[r], BLUE)
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

fn cp_row(mgr: &Mgr, id: u64, color: &str) -> (String, usize) {
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
