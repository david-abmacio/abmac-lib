//! MPSM (Multiple Producers, Multiple Mergers) support.
//!
//! `MergerHandle` wraps a [`FanInSpout`] over a subset of handoff slots,
//! enabling parallel collection across M merger threads. Created via
//! [`WorkerPool::with_mergers()`] (scoped, safe) or
//! [`WorkerPool::mergers_unchecked()`] (unsafe, unscoped).

use spout::Spout;

use super::collector::Collector;
use super::fan_in::FanInSpout;

/// A merger that owns a subset of handoff slots.
///
/// Each merger wraps a [`FanInSpout`] over its assigned slot subset.
/// Call [`flush()`](Self::flush) to drain assigned handoff slots into
/// the collector.
///
/// `MergerHandle` is [`Send`] when the collector is `Send`, enabling
/// use with [`std::thread::scope`] for parallel merging.
///
/// Created via [`WorkerPool::with_mergers()`] (scoped, safe) or
/// [`WorkerPool::mergers_unchecked()`] (unsafe, unscoped).
pub struct MergerHandle<T, const N: usize, S, C>
where
    S: Spout<T, Error = core::convert::Infallible>,
    C: Collector<T>,
{
    fan_in: FanInSpout<T, N, S, C>,
    merger_id: usize,
}

impl<T, const N: usize, S, C> MergerHandle<T, N, S, C>
where
    S: Spout<T, Error = core::convert::Infallible>,
    C: Collector<T>,
{
    /// Create a new merger handle.
    pub(crate) fn new(fan_in: FanInSpout<T, N, S, C>, merger_id: usize) -> Self {
        Self { fan_in, merger_id }
    }

    /// Drain all assigned handoff slots into the collector.
    pub fn flush(&mut self) -> Result<(), C::Error> {
        self.fan_in.flush()
    }

    /// This merger's index.
    #[inline]
    #[must_use]
    pub fn merger_id(&self) -> usize {
        self.merger_id
    }

    /// Number of handoff slots assigned to this merger.
    #[inline]
    #[must_use]
    pub fn num_slots(&self) -> usize {
        self.fan_in.num_slots()
    }

    /// Reference to the collector.
    #[inline]
    pub fn collector(&self) -> &C {
        self.fan_in.collector()
    }

    /// Mutable reference to the collector.
    #[inline]
    pub fn collector_mut(&mut self) -> &mut C {
        self.fan_in.collector_mut()
    }

    /// Consume and return the collector.
    #[inline]
    pub fn into_collector(self) -> C {
        self.fan_in.into_collector()
    }
}

// SAFETY: MergerHandle is Send when FanInSpout is Send (which requires
// C: Send). The raw pointer validity invariant is maintained by the
// scoped API or the unsafe constructor's contract.
unsafe impl<T, const N: usize, S, C> Send for MergerHandle<T, N, S, C>
where
    S: Spout<T, Error = core::convert::Infallible>,
    C: Collector<T> + Send,
{
}
