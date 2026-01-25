//! A no_std ring buffer that spills overflow to a sink.

#![no_std]

pub use spill_ring_core::*;

#[cfg(feature = "macros")]
pub use spill_ring_macros::{FromBytes, ToBytes};

/// Prelude for convenient imports.
pub mod prelude {
    pub use crate::{
        ByteSerializer, BytesError, CollectSink, DropSink, Flush, FnFlushSink, FnSink, FromBytes,
        RingConsumer, RingProducer, RingTrait, Sink, SpillRing, SpillRingIter, SpillRingIterMut,
        ToBytes, ViewBytes, sink,
    };
}
