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

#[path = "common/mod.rs"]
#[allow(unused)]
mod common;

use common::*;
use pebble::prelude::*;
use std::io::Write;
use std::thread;
use std::time::Duration;

const DELAY: Duration = Duration::from_millis(1400);
const HOT_CAPACITY: usize = 3;

// ── Checkpoint with real rebuild logic ──────────────────────────────────────
//
// Unlike the other examples, this checkpoint knows how to recompute
// its value from its parent. This is what makes `mgr.rebuild()` work.
//
// Layout:
//   #1 = 100   (root — no deps)
//   #2 = 300   depends on #1:  100 * 3
//   #3 = 488   depends on #2:  (300 - 56) * 2
//   #4 = 125   depends on #3:  (488 + 12) / 4

#[derive(Clone, Debug)]
struct RebuildCheckpoint {
    id: u64,
    value: u64,
}

impl Checkpointable for RebuildCheckpoint {
    type Id = u64;
    type RebuildError = &'static str;

    fn checkpoint_id(&self) -> u64 {
        self.id
    }

    fn compute_from_dependencies(
        base: Self,
        deps: &HashMap<Self::Id, &Self>,
    ) -> core::result::Result<Self, Self::RebuildError> {
        if deps.is_empty() {
            return Ok(base);
        }
        // Find the parent — there's exactly one dependency per checkpoint in this pipeline
        let (_, parent) = deps.iter().next().ok_or("missing parent dependency")?;
        let value = match base.id {
            2 => parent.value * 3,
            3 => (parent.value - 56) * 2,
            4 => (parent.value + 12) / 4,
            _ => return Err("unknown checkpoint"),
        };
        Ok(RebuildCheckpoint { id: base.id, value })
    }
}

struct RebuildSer;

impl CheckpointSerializer<RebuildCheckpoint> for RebuildSer {
    type Error = &'static str;

    fn serialize(&self, cp: &RebuildCheckpoint) -> core::result::Result<Vec<u8>, Self::Error> {
        let mut b = Vec::with_capacity(16);
        b.extend_from_slice(&cp.id.to_be_bytes());
        b.extend_from_slice(&cp.value.to_be_bytes());
        Ok(b)
    }

    fn deserialize(&self, b: &[u8]) -> core::result::Result<RebuildCheckpoint, Self::Error> {
        if b.len() < 16 {
            return Err("too small");
        }
        let id = u64::from_be_bytes(b[0..8].try_into().unwrap());
        let value = u64::from_be_bytes(b[8..16].try_into().unwrap());
        Ok(RebuildCheckpoint { id, value })
    }
}

type RebuildMgr = PebbleManager<
    RebuildCheckpoint,
    DirectStorage<InMemoryStorage<u64, u128, 16>, RebuildSer>,
    NoWarm,
>;

// ── Actions ─────────────────────────────────────────────────────────────────

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

// ── Main ────────────────────────────────────────────────────────────────────
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

fn main() {
    let cold = DirectStorage::new(InMemoryStorage::<u64, u128, 16>::new(), RebuildSer);
    let mut mgr = RebuildMgr::new(cold, NoWarm, Strategy::default(), HOT_CAPACITY);
    let mut val: u64 = 0;
    let mut step_n: u64 = 0;
    let mut flow: Vec<FlowNode> = Vec::new();
    let mut last_action_msg: Option<String> = None;

    #[rustfmt::skip]
    let steps: Vec<Step> = vec![
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
        Step { label: "  << load #1 from cold",          action: Action::Load { id: 1 } },
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
                let deps: Vec<u64> = dep.iter().copied().collect();
                mgr.add(
                    RebuildCheckpoint {
                        id: *id,
                        value: val,
                    },
                    &deps,
                )
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
            render(&mgr, &steps, Some(i), val, step_n, &flow, &last_action_msg)
        );
        std::io::stdout().flush().ok();
        thread::sleep(DELAY);
    }

    print!("{SHOW_CURSOR}");
    let s = mgr.stats();
    println!(
        "\n  {BOLD}Done.{RESET} {DIM}Checkpoints: {}  I/O: {}  Utilization: {:.0}%{RESET}\n",
        s.checkpoints_added(),
        s.io_operations(),
        s.hot_utilization() * 100.0,
    );
}

// ── Render ──────────────────────────────────────────────────────────────────

fn render(
    mgr: &RebuildMgr,
    steps: &[Step],
    current: Option<usize>,
    val: u64,
    step_n: u64,
    flow: &[FlowNode],
    action_msg: &Option<String>,
) -> String {
    let mut buf = String::with_capacity(4096);

    // Title
    row!(
        buf,
        "  {BOLD}Pebble \u{2014} Rebuild from Dependencies{RESET}"
    );
    row!(buf);

    // Step panel
    let labels: Vec<&str> = steps.iter().map(|s| s.label).collect();
    render_step_panel(&mut buf, &labels, current, |i| match &steps[i].action {
        Action::Save { .. } => GREEN,
        Action::Load { .. } => BLUE,
        Action::Rebuild { .. } => YELLOW,
        _ => DIM,
    });

    // Current value
    if step_n > 0 {
        row!(buf, "  {BOLD}x = {val}{RESET}  {DIM}(step {step_n}){RESET}");
    } else {
        row!(buf, "  {DIM}x = ---{RESET}");
    }
    row!(buf);

    // Flow chart
    render_flow_chart(&mut buf, flow, |id| mgr.is_hot(id));
    row!(buf);

    // Tier boxes
    let mut hot = Vec::new();
    let mut cold = Vec::new();
    for id in 1..=10u64 {
        if mgr.is_hot(id) {
            hot.push(id);
        } else if mgr.is_in_storage(id) {
            cold.push(id);
        }
    }
    let stats = mgr.stats();
    render_tier_boxes(
        &mut buf,
        &hot,
        &cold,
        stats.red_pebble_count(),
        HOT_CAPACITY,
        |id, color| {
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
        },
    );
    row!(buf);

    // Action message
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

    // Stats
    row!(
        buf,
        "  {DIM}I/O: {}    Checkpoints: {}    Utilization: {:.0}%{RESET}",
        stats.io_operations(),
        stats.checkpoints_added(),
        stats.hot_utilization() * 100.0
    );
    row!(buf);

    buf
}
