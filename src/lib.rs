//! A no_std ring buffer that spills overflow to a sink.

#![cfg_attr(not(feature = "std"), no_std)]

pub use spill_ring_core::*;

#[cfg(feature = "std")]
pub use spill_ring_core::WorkerPool;

#[cfg(feature = "macros")]
pub use spill_ring_macros::{FromBytes, ToBytes};

/// Prelude for convenient imports.
pub mod prelude {
    pub use crate::{
        BatchSink, ByteSerializer, BytesError, CollectSink, DropSink, Flush, FnFlushSink, FnSink,
        FromBytes, ReduceSink, RingConsumer, RingProducer, RingTrait, Sink, SpillRing,
        SpillRingIter, SpillRingIterMut, ToBytes, ViewBytes, sink,
    };

    #[cfg(feature = "std")]
    pub use crate::ChannelSink;
}
