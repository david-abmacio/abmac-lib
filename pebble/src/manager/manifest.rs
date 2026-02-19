//! Append-only WAL manifest for tracking checkpoint lifecycle.
//!
//! The manifest is a SpillRing that records evictions to cold storage
//! and tombstones for removed checkpoints. Together, hot + manifest =
//! complete picture: replay the log to determine the live set.
//!
//! The manifest is never mutated — tombstones are appended, never
//! deleted. A future compaction pass can replay the log to identify
//! garbage in cold storage.
//!
//! Write-ahead: eviction entries are written BEFORE `cold.store()`.

use core::convert::Infallible;

use spill_ring::SpillRing;
use spout::Spout;

/// A WAL entry recording a checkpoint eviction or removal.
///
/// Eviction entries carry dependencies so the full DAG can be
/// reconstructed from hot + manifest without scanning cold storage
/// metadata. Tombstone entries mark a checkpoint as removed — the
/// serialized data may still exist in cold storage until a
/// compaction pass cleans it up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManifestEntry<CId: Copy + Default, const MAX_DEPS: usize = 8> {
    /// The checkpoint this entry refers to.
    pub checkpoint_id: CId,
    /// Dependencies at eviction time (snapshot from the DAG).
    /// Zeroed for tombstone entries.
    pub dependencies: [CId; MAX_DEPS],
    /// Number of valid entries in `dependencies`.
    pub dep_count: u8,
    /// Monotonically increasing sequence number. Gaps indicate lost entries.
    pub seq: u64,
    /// `true` if this entry marks a removal (tombstone), `false` for eviction.
    pub tombstone: bool,
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
    pub(crate) fn new(spout: S) -> Self {
        Self {
            ring: SpillRing::builder().spout(spout).build(),
            seq: 0,
        }
    }

    /// Record a checkpoint eviction to cold storage.
    ///
    /// Called BEFORE `cold.store()` — write-ahead semantics.
    ///
    /// Returns [`StorageError::TooManyDependencies`] if
    /// `dependencies.len()` exceeds `MAX_DEPS`.
    pub(crate) fn record(
        &mut self,
        checkpoint_id: CId,
        dependencies: &[CId],
    ) -> Result<(), crate::storage::StorageError> {
        debug_assert!(
            dependencies.len() <= MAX_DEPS,
            "manifest record: {} dependencies exceeds MAX_DEPS={}",
            dependencies.len(),
            MAX_DEPS,
        );

        if dependencies.len() > MAX_DEPS {
            return Err(crate::storage::StorageError::TooManyDependencies {
                max: MAX_DEPS,
                count: dependencies.len(),
            });
        }

        let mut deps = [CId::default(); MAX_DEPS];
        deps[..dependencies.len()].copy_from_slice(dependencies);

        let entry = ManifestEntry {
            checkpoint_id,
            dependencies: deps,
            dep_count: dependencies.len() as u8,
            seq: self.seq,
            tombstone: false,
        };
        self.seq += 1;
        self.ring.push_mut(entry);
        Ok(())
    }

    /// Record that a cold checkpoint was removed.
    ///
    /// Appends a tombstone entry. The serialized data remains in cold
    /// storage until a compaction pass; this entry marks it as garbage.
    pub(crate) fn record_tombstone(&mut self, checkpoint_id: CId) {
        let entry = ManifestEntry {
            checkpoint_id,
            dependencies: [CId::default(); MAX_DEPS],
            dep_count: 0,
            seq: self.seq,
            tombstone: true,
        };
        self.seq += 1;
        self.ring.push_mut(entry);
    }

    /// Flush buffered entries to the spout.
    pub(crate) fn flush(&mut self) {
        let _ = self.ring.flush();
    }

    /// Current sequence number (next entry will have this seq).
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Borrow the underlying spout.
    pub fn spout(&self) -> &S {
        self.ring.spout()
    }

    /// Mutably borrow the underlying spout.
    pub fn spout_mut(&mut self) -> &mut S {
        self.ring.spout_mut()
    }
}
