#![no_std]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

mod impls;
mod traits;

#[cfg(test)]
mod tests;

pub use impls::*;
pub use traits::{Flush, Spout};

#[cfg(feature = "std")]
pub use impls::{ChannelSpout, SyncChannelSpout};

#[cfg(feature = "bytecast")]
pub use bytecast::{FromBytes, FromBytesExt, ToBytes, ToBytesExt};
