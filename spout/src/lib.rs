#![no_std]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

mod impls;
mod traits;

#[cfg(test)]
mod tests;

pub use impls::*;
pub use traits::{Flush, FlushFn, Spout};

#[cfg(feature = "bytecast")]
pub use bytecast::{FromBytes, FromBytesExt, ToBytes, ToBytesExt};
