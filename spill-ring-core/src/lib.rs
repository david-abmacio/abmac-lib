//! Core implementation for spill_ring.

#![cfg_attr(not(feature = "std"), no_std)]
#![warn(missing_docs)]

pub mod bytes;
mod index;
mod iter;
mod mpsc;
mod ring;
mod sink;
mod traits;

#[cfg(test)]
mod tests;

pub use bytes::{ByteSerializer, BytesError, FromBytes, ToBytes, ViewBytes};
pub use iter::{SpillRingIter, SpillRingIterMut};
#[cfg(feature = "std")]
pub use mpsc::WorkerPool;
pub use mpsc::{Consumer, MpscRing, Producer, collect_producers};
pub use ring::SpillRing;
#[cfg(feature = "std")]
pub use sink::ChannelSink;
pub use sink::{
    BatchSink, CollectSink, DropSink, Flush, FnFlushSink, FnSink, ProducerSink, ReduceSink, Sink,
    sink,
};
pub use traits::{RingConsumer, RingInfo, RingProducer, RingTrait};
