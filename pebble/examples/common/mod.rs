//! Shared types and helpers for the animated pebble examples.

use pebble::prelude::*;

// ── ANSI escapes ────────────────────────────────────────────────────────────

pub const RED: &str = "\x1b[31m";
pub const BLUE: &str = "\x1b[34m";
pub const GREEN: &str = "\x1b[32m";
pub const YELLOW: &str = "\x1b[33m";
pub const MAGENTA: &str = "\x1b[35m";
pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";
pub const RESET: &str = "\x1b[0m";
pub const HIDE_CURSOR: &str = "\x1b[?25l";
pub const SHOW_CURSOR: &str = "\x1b[?25h";

// ── Terminal helpers ────────────────────────────────────────────────────────

pub fn clear_screen() {
    std::process::Command::new("clear").status().ok();
}

pub fn wait_for_key() {
    let mut tty = std::fs::File::open("/dev/tty").unwrap();
    std::process::Command::new("stty")
        .args(["raw", "-echo"])
        .stdin(std::fs::File::open("/dev/tty").unwrap())
        .status()
        .ok();
    use std::io::Read;
    let mut buf = [0u8; 1];
    let _ = tty.read(&mut buf);
    std::process::Command::new("stty")
        .args(["cooked", "echo"])
        .stdin(std::fs::File::open("/dev/tty").unwrap())
        .status()
        .ok();
}

#[macro_export]
macro_rules! row {
    ($b:expr) => {{ $b.push('\n'); }};
    ($b:expr, $($a:tt)*) => {{
        use std::fmt::Write as _;
        writeln!($b, $($a)*).unwrap();
    }};
}

// ── Checkpoint type ─────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct Checkpoint {
    pub id: u64,
    pub value: u64,
}

impl Checkpointable for Checkpoint {
    type Id = u64;
    type RebuildError = &'static str;

    fn checkpoint_id(&self) -> u64 {
        self.id
    }

    fn compute_from_dependencies(
        base: Self,
        _: &HashMap<Self::Id, &Self>,
    ) -> core::result::Result<Self, Self::RebuildError> {
        Ok(base)
    }
}

// ── Serializer ──────────────────────────────────────────────────────────────

pub struct Ser;

impl CheckpointSerializer<Checkpoint> for Ser {
    type Error = &'static str;

    fn serialize(&self, cp: &Checkpoint) -> core::result::Result<Vec<u8>, Self::Error> {
        let mut b = Vec::with_capacity(16);
        b.extend_from_slice(&cp.id.to_be_bytes());
        b.extend_from_slice(&cp.value.to_be_bytes());
        Ok(b)
    }

    fn deserialize(&self, b: &[u8]) -> core::result::Result<Checkpoint, Self::Error> {
        if b.len() < 16 {
            return Err("too small");
        }
        let id = u64::from_be_bytes(b[0..8].try_into().unwrap());
        let value = u64::from_be_bytes(b[8..16].try_into().unwrap());
        Ok(Checkpoint { id, value })
    }
}

// ── Manager type alias ──────────────────────────────────────────────────────

pub type Mgr =
    PebbleManager<Checkpoint, DirectStorage<InMemoryStorage<u64, u128, 16>, Ser>, NoWarm>;

pub fn new_manager(hot_capacity: usize) -> Mgr {
    let cold = DirectStorage::new(InMemoryStorage::<u64, u128, 16>::new(), Ser);
    Mgr::new(cold, NoWarm, Strategy::default(), hot_capacity)
}

// ── Step panel ──────────────────────────────────────────────────────────────

/// Render the step panel box.
///
/// - `labels`: the text for each step
/// - `current`: index of the active step (if any)
/// - `color_fn(index)`: ANSI color for step `index` (used for completed checkmarks
///   and the current-step arrow; future steps are always dimmed)
pub fn render_step_panel(
    buf: &mut String,
    labels: &[&str],
    current: Option<usize>,
    color_fn: impl Fn(usize) -> &'static str,
) {
    let width = labels.iter().map(|l| l.len()).max().unwrap_or(0) + 6;
    let border = "\u{2500}".repeat(width);
    row!(buf, "  {DIM}\u{250c}{border}\u{2510}{RESET}");
    for (i, label) in labels.iter().enumerate() {
        let color = color_fn(i);
        match current {
            Some(c) if i < c => row!(buf, "  {color}  \u{2713} {label}{RESET}"),
            Some(c) if i == c => row!(buf, "  {color}{BOLD}  > {label}{RESET}"),
            _ => row!(buf, "  {DIM}    {label}{RESET}"),
        };
    }
    row!(buf, "  {DIM}\u{2514}{border}\u{2518}{RESET}");
    row!(buf);
}

// ── Tier display helpers ────────────────────────────────────────────────────

/// Collect hot and cold checkpoint IDs by scanning a range.
pub fn collect_tiers(mgr: &Mgr, max_id: u64) -> (Vec<u64>, Vec<u64>) {
    let mut hot = Vec::new();
    let mut cold = Vec::new();
    for id in 1..=max_id {
        if mgr.is_hot(id) {
            hot.push(id);
        } else if mgr.is_in_storage(id) {
            cold.push(id);
        }
    }
    (hot, cold)
}

/// Format a checkpoint entry for a tier box. Returns (colored_string, visible_length).
pub fn tier_entry(mgr: &Mgr, id: u64, color: &str) -> (String, usize) {
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

/// Render the HOT / COLD tier boxes.
///
/// Takes pre-collected data so it works with any manager type:
/// - `hot` / `cold`: checkpoint IDs in each tier
/// - `used`: number of red pebbles (for the header)
/// - `hot_capacity`: max hot slots
/// - `entry_fn(id, color)`: formats one row, returns (colored_string, visible_length)
pub fn render_tier_boxes(
    buf: &mut String,
    hot: &[u64],
    cold: &[u64],
    used: usize,
    hot_capacity: usize,
    entry_fn: impl Fn(u64, &str) -> (String, usize),
) {
    // Size the box to fit the widest entry (minimum 18 for the header).
    let max_entry = hot
        .iter()
        .map(|id| entry_fn(*id, "").1)
        .chain(cold.iter().map(|id| entry_fn(*id, "").1))
        .max()
        .unwrap_or(0);
    let inner: usize = max_entry.max(18);

    let hot_hdr = format!("{RED}{BOLD}HOT{RESET} {DIM}{used}/{hot_capacity}");
    let hot_hdr_vis = 4 + used.to_string().len() + 1 + hot_capacity.to_string().len();
    let cold_hdr = format!("{BLUE}{BOLD}COLD{RESET}");
    let cold_hdr_vis: usize = 4;

    let hf = "\u{2500}".repeat(inner.saturating_sub(hot_hdr_vis + 1));
    let cf = "\u{2500}".repeat(inner.saturating_sub(cold_hdr_vis + 1));

    row!(
        buf,
        "  \u{250c}\u{2500} {hot_hdr} {hf}{RESET}\u{2510}    \u{250c}\u{2500} {cold_hdr} {DIM}{cf}{RESET}\u{2510}"
    );

    let max_rows = hot_capacity.max(cold.len()).max(1);
    for r in 0..max_rows {
        let (hc, hv) = if r < hot.len() {
            entry_fn(hot[r], RED)
        } else {
            (String::new(), 0)
        };
        let (cc, cv) = if r < cold.len() {
            entry_fn(cold[r], BLUE)
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
}

// ── Flow chart ──────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct FlowNode {
    pub value: u64,
    pub cp_id: Option<u64>,
}

/// Render a horizontal flow chart of computed values with checkpoint labels beneath.
///
/// `is_hot(id)` tells the renderer which color to use for each checkpoint.
pub fn render_flow_chart(buf: &mut String, flow: &[FlowNode], is_hot: impl Fn(u64) -> bool) {
    if flow.is_empty() {
        return;
    }
    let mut top = String::from("  ");
    let mut bot = String::from("  ");
    for (i, node) in flow.iter().enumerate() {
        let v = format!("{}", node.value);
        let colored = if let Some(id) = node.cp_id {
            if is_hot(id) {
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
            if is_hot(id) {
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
            top.push_str(&format!(" {DIM}\u{2192}{RESET} "));
            bot.push_str("   ");
        }
    }
    row!(buf, "{top}");
    row!(buf, "{bot}");
}
