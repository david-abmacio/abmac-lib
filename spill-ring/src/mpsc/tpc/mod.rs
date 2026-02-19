//! TPC (Thread-Per-Core) std-only modules.
//!
//! Everything in this subtree requires `std`. The parent `mpsc` module
//! gates this with a single `#[cfg(feature = "std")]` on the `mod`
//! declaration — no per-item feature flags here.

pub(crate) mod collector;
pub(crate) mod fan_in;
pub(crate) mod handoff;
pub(crate) mod merger;
pub(crate) mod pool;
pub(crate) mod streaming;
pub(crate) mod sync;

pub use collector::{Collector, SequencedCollector, UnorderedCollector};
pub use fan_in::FanInSpout;
pub use merger::MergerHandle;
pub use pool::{PoolBuilder, WorkerPanic, WorkerPool};
pub use streaming::{StreamingFanIn, StreamingMergers};
