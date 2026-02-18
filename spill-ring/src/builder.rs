//! Builder pattern for constructing ring buffers.

use core::marker::PhantomData;

use spout::{DropSpout, Spout};

use crate::SpillRing;

/// Builder for constructing a [`SpillRing`].
///
/// Created via [`SpillRing::builder()`]. Configure options with chained
/// methods, then call [`.build()`](Self::build) to construct the ring.
///
/// # Example
///
/// ```
/// use spill_ring::SpillRing;
///
/// // Default: warmed, DropSpout
/// let ring = SpillRing::<u64, 256>::builder().build();
///
/// // Cold (no cache warming)
/// let ring = SpillRing::<u64, 256>::builder().cold().build();
/// ```
pub struct SpillRingBuilder<
    T,
    const N: usize,
    S: Spout<T, Error = core::convert::Infallible> = DropSpout,
> {
    sink: S,
    warm: bool,
    _marker: PhantomData<T>,
}

impl<T, const N: usize> SpillRingBuilder<T, N, DropSpout> {
    pub(crate) fn new() -> Self {
        Self {
            sink: DropSpout,
            warm: true,
            _marker: PhantomData,
        }
    }
}

impl<T, const N: usize, S: Spout<T, Error = core::convert::Infallible>> SpillRingBuilder<T, N, S> {
    /// Set a custom spout for handling evicted items.
    pub fn sink<S2: Spout<T, Error = core::convert::Infallible>>(
        self,
        sink: S2,
    ) -> SpillRingBuilder<T, N, S2> {
        SpillRingBuilder {
            sink,
            warm: self.warm,
            _marker: PhantomData,
        }
    }

    /// Disable cache warming.
    ///
    /// By default, the builder warms the ring (touches all slots to pull them
    /// into L1/L2 cache). Use this for constrained environments where the
    /// warming overhead is unacceptable.
    #[must_use]
    pub fn cold(mut self) -> Self {
        self.warm = false;
        self
    }

    /// Build the [`SpillRing`].
    pub fn build(self) -> SpillRing<T, N, S> {
        if self.warm {
            SpillRing::with_sink(self.sink)
        } else {
            SpillRing::with_sink_cold(self.sink)
        }
    }
}
