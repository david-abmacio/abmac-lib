//! Core implementation for spill_ring.

#![no_std]
#![warn(missing_docs)]

pub mod bytes;
mod index;
mod iter;
mod ring;
mod sink;
mod traits;

#[cfg(test)]
mod tests;

pub use bytes::{ByteSerializer, BytesError, FromBytes, ToBytes, ViewBytes};
pub use iter::{SpillRingIter, SpillRingIterMut};
pub use ring::SpillRing;
pub use sink::{sink, DropSink, Flush, FnFlushSink, FnSink, Sink};
pub use traits::{RingConsumer, RingProducer, RingTrait};
