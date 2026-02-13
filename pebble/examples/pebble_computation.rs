//! Animated computation pipeline with selective checkpointing.
//!
//! A pipeline runs many computation steps, but only some are
//! checkpointed. The manager keeps recent checkpoints in hot memory
//! and evicts older ones to cold storage. When a rollback happens,
//! the manager loads the nearest checkpoint from cold — anything
//! between checkpoints must be recomputed.
//!
//! Run with: cargo run -p pebble --example pebble_computation

#[allow(unused)]
mod common;

use common::*;
use std::io::Write;
use std::thread;
use std::time::Duration;

const DELAY: Duration = Duration::from_millis(1200);
const HOT_CAPACITY: usize = 3;

// ── Actions ─────────────────────────────────────────────────────────────────

enum Action {
    Compute { step: u64, value: u64 },
    Save { id: u64 },
    Rollback { id: u64 },
}

struct Step {
    label: &'static str,
    action: Action,
}

// ── Main ────────────────────────────────────────────────────────────────────

fn main() {
    let mut mgr = new_manager(HOT_CAPACITY);
    let mut val: u64 = 0;
    let mut step_n: u64 = 0;
    let mut flow: Vec<FlowNode> = Vec::new();
    let mut restored = false;

    #[rustfmt::skip]
    let steps: Vec<Step> = vec![
        Step { label: "x = 100",                       action: Action::Compute { step: 1, value: 100 } },
        Step { label: "x = x * 3 = 300",               action: Action::Compute { step: 2, value: 300 } },
        Step { label: "  >> save checkpoint #1",        action: Action::Save { id: 1 } },
        Step { label: "x = x - 56 = 244",              action: Action::Compute { step: 3, value: 244 } },
        Step { label: "x = x * 2 = 488",               action: Action::Compute { step: 4, value: 488 } },
        Step { label: "  >> save checkpoint #2",        action: Action::Save { id: 2 } },
        Step { label: "x = x + 12 = 500",              action: Action::Compute { step: 5, value: 500 } },
        Step { label: "x = x / 4 = 125",               action: Action::Compute { step: 6, value: 125 } },
        Step { label: "  >> save checkpoint #3",        action: Action::Save { id: 3 } },
        Step { label: "x = x + 75 = 200",              action: Action::Compute { step: 7, value: 200 } },
        Step { label: "x = x * 5 = 1000",              action: Action::Compute { step: 8, value: 1000 } },
        Step { label: "  >> save checkpoint #4",        action: Action::Save { id: 4 } },
        Step { label: "  << rollback to #2 (488)",      action: Action::Rollback { id: 2 } },
        Step { label: "x = x - 8 = 480",               action: Action::Compute { step: 9, value: 480 } },
        Step { label: "  >> save checkpoint #5",        action: Action::Save { id: 5 } },
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
                mgr.add(
                    Checkpoint {
                        id: *id,
                        value: val,
                    },
                    &[],
                )
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
        s.checkpoints_added(),
        s.io_operations(),
        s.hot_utilization() * 100.0,
    );
}

// ── Render ──────────────────────────────────────────────────────────────────

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

    // Step panel
    let labels: Vec<&str> = steps.iter().map(|s| s.label).collect();
    render_step_panel(&mut buf, &labels, current, |i| match &steps[i].action {
        Action::Save { .. } => GREEN,
        Action::Rollback { .. } => BLUE,
        _ => DIM,
    });

    // Current value
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

    // Flow chart
    render_flow_chart(&mut buf, flow, |id| mgr.is_hot(id));
    row!(buf);

    // Tier boxes
    let (hot, cold) = collect_tiers(mgr, 20);
    let stats = mgr.stats();
    render_tier_boxes(
        &mut buf,
        &hot,
        &cold,
        stats.red_pebble_count(),
        HOT_CAPACITY,
        |id, color| tier_entry(mgr, id, color),
    );
    row!(buf);
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
