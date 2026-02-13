//! Umbrella re-export of ABMAC derive macros.
//!
//! Enable feature flags to pull in derive macros from their respective crates:
//!
//! - `bytecast` — `ToBytes`, `FromBytes`
#![no_std]

#[cfg(feature = "bytecast")]
pub use bytecast_macros::{FromBytes, ToBytes};

#[cfg(test)]
mod tests;
