//! A no_std ring buffer that spills overflow to a sink.

#![no_std]

pub use spill_ring_core::*;

#[cfg(feature = "macros")]
pub use spill_ring_macros::{FromBytes, ToBytes};

/// Prelude for convenient imports.
pub mod prelude {
    pub use crate::{
        ByteSerializer, BytesError, FromBytes, ToBytes, ViewBytes,
        SpillRing, SpillRingIter, SpillRingIterMut,
        Sink, DropSink, FnSink, FnFlushSink, Flush, sink,
        RingProducer, RingConsumer, RingTrait,
    };
}
