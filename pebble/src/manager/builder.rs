//! Typestate builder for [`PebbleManager`].
//!
//! The builder enforces at compile time that all required components
//! (cold tier, warm tier, log sink) are provided before construction.
//! Optional settings (strategy, hot capacity) have sensible defaults.
//!
//! # Example
//!
//! ```ignore
//! use pebble::{Checkpoint, PebbleBuilder, InMemoryStorage, DirectStorage, NoWarm};
//! use spout::DropSpout;
//!
//! #[derive(Clone, Debug, Checkpoint)]
//! struct MyCp {
//!     #[checkpoint(id)]
//!     id: u64,
//! }
//!
//! let mut manager = PebbleBuilder::new()
//!     .cold(DirectStorage::new(InMemoryStorage::<u64, u128, 8>::new()))
//!     .warm(NoWarm)
//!     .log(DropSpout)
//!     .hot_capacity(4)
//!     .build::<MyCp>();
//! ```

use core::convert::Infallible;

use spout::Spout;

use crate::strategy::Strategy;

use super::cold::ColdTier;
use super::manifest::{Manifest, ManifestEntry};
use super::pebble_manager::PebbleManager;
use super::traits::Checkpointable;
use super::warm::WarmTier;

/// Default hot tier capacity when no hint is provided.
const DEFAULT_HOT_CAPACITY: usize = 16;

/// Marker type: required component not yet provided.
///
/// Used as the default for `PebbleBuilder`'s generic parameters.
/// Calling `.build()` with any `Missing` slot will fail to compile
/// because `Missing` does not implement [`ColdTier`], [`WarmTier`],
/// or [`Spout`].
#[derive(Debug, Clone, Copy)]
pub struct Missing;

/// Typestate builder for [`PebbleManager`].
///
/// Generic parameters track which required components have been
/// provided:
///
/// - `C` — cold tier (starts as [`Missing`])
/// - `W` — warm tier (starts as [`Missing`])
/// - `L` — log sink (starts as [`Missing`])
///
/// Call [`.cold()`](Self::cold), [`.warm()`](Self::warm), and
/// [`.log()`](Self::log) to provide all three,
/// then [`.build::<T>()`](Self::build) to construct the manager.
pub struct PebbleBuilder<C = Missing, W = Missing, L = Missing> {
    cold: C,
    warm: W,
    log: L,
    strategy: Strategy,
    hot_capacity: usize,
}

// ── Constructor ─────────────────────────────────────────────────────────

impl PebbleBuilder {
    /// Create a new builder. All required components start as [`Missing`].
    pub fn new() -> Self {
        Self {
            cold: Missing,
            warm: Missing,
            log: Missing,
            strategy: Strategy::default(),
            hot_capacity: DEFAULT_HOT_CAPACITY,
        }
    }
}

impl Default for PebbleBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ── Required: cold tier ─────────────────────────────────────────────────

impl<W, L> PebbleBuilder<Missing, W, L> {
    /// Set the cold tier (required).
    pub fn cold<C>(self, cold: C) -> PebbleBuilder<C, W, L> {
        PebbleBuilder {
            cold,
            warm: self.warm,
            log: self.log,
            strategy: self.strategy,
            hot_capacity: self.hot_capacity,
        }
    }
}

// ── Required: warm tier ─────────────────────────────────────────────────

impl<C, L> PebbleBuilder<C, Missing, L> {
    /// Set the warm tier (required).
    ///
    /// Use [`NoWarm`](super::warm::NoWarm) for a zero-cost tier that
    /// sends evictions straight to cold storage.
    pub fn warm<W>(self, warm: W) -> PebbleBuilder<C, W, L> {
        PebbleBuilder {
            cold: self.cold,
            warm,
            log: self.log,
            strategy: self.strategy,
            hot_capacity: self.hot_capacity,
        }
    }
}

// ── Required: log sink ──────────────────────────────────────────────────

impl<C, W> PebbleBuilder<C, W, Missing> {
    /// Set the log sink (required).
    ///
    /// The sink receives WAL entries when the manifest ring overflows.
    /// Use [`DropSpout`](spout::DropSpout) if you don't need manifest
    /// persistence.
    pub fn log<S>(self, spout: S) -> PebbleBuilder<C, W, S> {
        PebbleBuilder {
            cold: self.cold,
            warm: self.warm,
            log: spout,
            strategy: self.strategy,
            hot_capacity: self.hot_capacity,
        }
    }
}

// ── Optional settings (always available) ────────────────────────────────

impl<C, W, L> PebbleBuilder<C, W, L> {
    /// Set the eviction strategy. Default: [`Strategy::default()`] (DAG).
    pub fn strategy(mut self, strategy: Strategy) -> Self {
        self.strategy = strategy;
        self
    }

    /// Set hot tier capacity (max red pebbles). Default: 16.
    pub fn hot_capacity(mut self, size: usize) -> Self {
        self.hot_capacity = size;
        self
    }

    /// Compute `hot_capacity` from expected total checkpoints.
    ///
    /// Sets `hot_capacity = sqrt(n)` (minimum 1). Users who know their
    /// workload size get optimal configuration without understanding
    /// pebble game theory.
    pub fn hint_total_checkpoints(mut self, n: usize) -> Self {
        self.hot_capacity = n.isqrt().max(1);
        self
    }
}

// ── Build ───────────────────────────────────────────────────────────────

impl<C, W, L> PebbleBuilder<C, W, L> {
    /// Build the [`PebbleManager`].
    ///
    /// All three required components (cold, warm, log) must be set or
    /// this will not compile. `hot_capacity` is clamped to a minimum of 1.
    ///
    /// `T` is the checkpoint type — inferred or specified via turbofish.
    pub fn build<T>(self) -> PebbleManager<T, C, W, L>
    where
        T: Checkpointable,
        C: ColdTier<T>,
        W: WarmTier<T>,
        L: Spout<ManifestEntry<T::Id>, Error = Infallible>,
    {
        let manifest = Manifest::new(self.log);
        PebbleManager::new(
            self.cold,
            self.warm,
            manifest,
            self.strategy,
            self.hot_capacity.max(1),
        )
    }

    /// Recover state from existing storage.
    ///
    /// Like [`build`](Self::build), but reconstructs the DAG and
    /// checkpoint registry from persisted cold-tier metadata. Requires
    /// a [`RecoverableColdTier`](super::cold::RecoverableColdTier).
    #[allow(clippy::type_complexity)]
    pub fn recover<T>(
        self,
    ) -> super::error::Result<
        (PebbleManager<T, C, W, L>, crate::storage::RecoveryResult),
        T::Id,
        C::Error,
    >
    where
        T: Checkpointable,
        C: super::cold::RecoverableColdTier<T>,
        W: WarmTier<T>,
        L: Spout<ManifestEntry<T::Id>, Error = Infallible>,
    {
        let manifest = Manifest::new(self.log);
        PebbleManager::recover(
            self.cold,
            self.warm,
            manifest,
            self.strategy,
            self.hot_capacity.max(1),
        )
    }
}
