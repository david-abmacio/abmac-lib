//! Core implementation for `spill_ring`.

#![cfg_attr(not(feature = "std"), no_std)]
#![warn(missing_docs)]

mod builder;
mod error;
mod index;
mod iter;
#[cfg(feature = "alloc")]
mod mpsc;
mod read;
mod ring;
mod traits;

#[cfg(test)]
mod tests;

pub use builder::SpillRingBuilder;
pub use error::PushError;
pub use iter::{SpillRingIter, SpillRingIterMut};
#[cfg(feature = "alloc")]
pub use mpsc::{Consumer, MpscRing, Producer, collect};
#[cfg(feature = "std")]
pub use mpsc::{PoolBuilder, WorkerPanic, WorkerPool};
pub use ring::{Drain, SpillRing};
pub use traits::{RingConsumer, RingInfo, RingProducer, RingTrait};
