//! Core implementation for spill_ring.

#![cfg_attr(not(feature = "std"), no_std)]
#![warn(missing_docs)]

mod builder;
mod index;
mod iter;
mod mpsc;
mod read;
mod ring;
mod spsc;
mod traits;

#[cfg(test)]
mod tests;

pub use builder::{SpillRingBuilder, SpscRingBuilder};
pub use iter::{SpillRingIter, SpillRingIterMut};
pub use mpsc::{Consumer, MpscRing, Producer, collect};
#[cfg(feature = "std")]
pub use mpsc::{PoolBuilder, WorkerPool};
pub use ring::{Drain, SpillRing};
#[cfg(feature = "std")]
pub use spsc::{SpscConsumer, SpscProducer};
pub use spsc::{SpscDrain, SpscRing, SpscRingIter, SpscRingIterMut};
pub use traits::{RingConsumer, RingInfo, RingProducer, RingTrait};
