//! Manifest verification: cross-reference DAG + hot against manifest entries.
//!
//! DAG + hot is the source of truth. The manifest is the verification
//! mechanism. Every DAG node should either be in hot or have a manifest
//! entry (was evicted to cold). A node in neither was lost.

use alloc::vec::Vec;
use core::convert::Infallible;

use hashbrown::HashSet;
use spout::Spout;

use super::cold::ColdTier;
use super::manifest::ManifestEntry;
use super::pebble_manager::PebbleManager;
use super::traits::Checkpointable;
use super::warm::WarmTier;

/// Result of verifying the manager's state against manifest entries.
#[derive(Debug)]
pub struct VerificationResult<CId: Copy + core::fmt::Debug> {
    /// DAG nodes not in hot, not in warm, and not in the manifest — lost.
    pub lost: Vec<CId>,
    /// Number of sequence gaps detected in the manifest.
    pub sequence_gaps: u64,
    /// Whether verification passed (no lost nodes, no gaps).
    pub is_consistent: bool,
}

impl<T, C, W, S> PebbleManager<T, C, W, S>
where
    T: Checkpointable,
    C: ColdTier<T>,
    W: WarmTier<T>,
    S: Spout<ManifestEntry<T::Id>, Error = Infallible>,
{
    /// Verify manager state against persisted manifest entries.
    ///
    /// The caller provides an iterator of manifest entries read back from
    /// wherever the spout drained to (file, database, etc.). For every
    /// node in the DAG, checks if it's in hot, warm, or has a manifest
    /// entry. Any node in none of those was lost.
    ///
    /// Also checks for sequence gaps in the manifest entries.
    ///
    /// **Important:** Call [`flush()`](Self::flush) before verifying.
    /// Entries buffered in the manifest ring have not yet drained to the
    /// spout, so verification without flushing may report false losses.
    pub fn verify<I>(&self, manifest_entries: I) -> VerificationResult<T::Id>
    where
        I: IntoIterator<Item = ManifestEntry<T::Id>>,
    {
        // Collect all checkpoint IDs that appear in manifest entries.
        let mut manifest_ids: HashSet<T::Id> = HashSet::new();
        let mut max_seq: Option<u64> = None;
        let mut entry_count: u64 = 0;

        for entry in manifest_entries {
            // Tombstoned checkpoints are removed — they don't count as
            // covering a DAG node.
            if !entry.tombstone {
                manifest_ids.insert(entry.checkpoint_id);
            }
            max_seq = Some(max_seq.map_or(entry.seq, |m: u64| m.max(entry.seq)));
            entry_count += 1;
        }

        // Also include entries still buffered in the ring (not yet drained).
        // The ring's seq counter tells us how many entries have been produced
        // total; entries in the ring haven't spilled to the spout yet.
        // We don't have access to ring contents here, but the caller should
        // flush the manifest before verifying for accurate results.

        // Detect sequence gaps: if max_seq is N, we expect N+1 entries
        // (seq 0 through N). Any shortfall is a gap.
        let sequence_gaps = match max_seq {
            Some(m) => (m + 1).saturating_sub(entry_count),
            None => 0,
        };

        // Check every DAG node: should be in hot, warm, or manifest.
        let mut lost = Vec::new();
        for &node_id in self.dag.node_ids() {
            if self.red_pebbles.contains_key(&node_id) {
                continue;
            }
            if self.warm.contains(node_id) {
                continue;
            }
            if manifest_ids.contains(&node_id) {
                continue;
            }
            lost.push(node_id);
        }

        let is_consistent = lost.is_empty() && sequence_gaps == 0;

        VerificationResult {
            lost,
            sequence_gaps,
            is_consistent,
        }
    }
}
