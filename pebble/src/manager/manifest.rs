//! WAL manifest for tracking checkpoint evictions to cold storage.
//!
//! The manifest is a SpillRing that records every eviction with
//! dependency metadata. Together, hot + manifest = complete picture:
//! every checkpoint is either in hot or has a manifest entry. A
//! checkpoint in neither was lost.
//!
//! Write-ahead: manifest entries are written BEFORE `cold.store()`.

use core::convert::Infallible;

use spill_ring::SpillRing;
use spout::Spout;

/// A WAL entry recording a checkpoint eviction to cold storage.
///
/// Carries dependencies so the full DAG can be reconstructed from
/// hot + manifest without scanning cold storage metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManifestEntry<CId: Copy + Default, const MAX_DEPS: usize = 8> {
    /// The checkpoint that was evicted to cold storage.
    pub checkpoint_id: CId,
    /// Dependencies at eviction time (snapshot from the DAG).
    pub dependencies: [CId; MAX_DEPS],
    /// Number of valid entries in `dependencies`.
    pub dep_count: u8,
    /// Monotonically increasing sequence number. Gaps indicate lost entries.
    pub seq: u64,
}

/// SpillRing-backed WAL manifest.
///
/// The manifest IS a SpillRing. Entries that overflow the ring drain
/// to the user-provided spout `S` automatically.
///
/// # Type Parameters
/// - `CId` — Checkpoint ID type
/// - `S` — Spout where entries drain on overflow
/// - `MAX_DEPS` — Maximum dependencies per entry
/// - `N` — Ring buffer capacity
pub struct Manifest<CId, S, const MAX_DEPS: usize = 8, const N: usize = 64>
where
    CId: Copy + Default,
    S: Spout<ManifestEntry<CId, MAX_DEPS>, Error = Infallible>,
{
    ring: SpillRing<ManifestEntry<CId, MAX_DEPS>, N, S>,
    seq: u64,
}

impl<CId, S, const MAX_DEPS: usize, const N: usize> Manifest<CId, S, MAX_DEPS, N>
where
    CId: Copy + Default,
    S: Spout<ManifestEntry<CId, MAX_DEPS>, Error = Infallible>,
{
    /// Create a new manifest draining to the given spout.
    pub fn new(spout: S) -> Self {
        Self {
            ring: SpillRing::with_sink(spout),
            seq: 0,
        }
    }

    /// Record a checkpoint eviction to cold storage.
    ///
    /// Called BEFORE `cold.store()` — write-ahead semantics.
    pub(crate) fn record(&mut self, checkpoint_id: CId, dependencies: &[CId]) {
        let mut deps = [CId::default(); MAX_DEPS];
        let dep_count = dependencies.len().min(MAX_DEPS);
        deps[..dep_count].copy_from_slice(&dependencies[..dep_count]);

        let entry = ManifestEntry {
            checkpoint_id,
            dependencies: deps,
            dep_count: dep_count as u8,
            seq: self.seq,
        };
        self.seq += 1;
        self.ring.push_mut(entry);
    }

    /// Flush buffered entries to the spout.
    pub(crate) fn flush(&mut self) {
        self.ring.flush();
    }

    /// Current sequence number (next entry will have this seq).
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Borrow the underlying spout.
    pub fn spout(&self) -> &S {
        self.ring.sink_ref()
    }

    /// Mutably borrow the underlying spout.
    pub fn spout_mut(&mut self) -> &mut S {
        self.ring.sink_mut()
    }
}
