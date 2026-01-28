//! Core implementation for spill_ring.

#![cfg_attr(not(feature = "std"), no_std)]
#![warn(missing_docs)]

mod index;
mod iter;
mod mpsc;
mod ring;
mod traits;

#[cfg(test)]
mod tests;

pub use bytecast::{ByteSerializer, BytesError, FromBytes, ToBytes, ViewBytes};
pub use spout::{
    BatchSink, CollectSink, DropSink, Flush, FnFlushSink, FnSink, ProducerSink, ReduceSink, Sink,
    sink,
};

#[cfg(feature = "std")]
pub use spout::ChannelSink;

pub use iter::{SpillRingIter, SpillRingIterMut};
#[cfg(feature = "std")]
pub use mpsc::WorkerPool;
pub use mpsc::{Consumer, MpscRing, Producer, collect};
pub use ring::SpillRing;
pub use traits::{RingConsumer, RingInfo, RingProducer, RingTrait};
