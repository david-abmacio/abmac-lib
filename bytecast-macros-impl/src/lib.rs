//! Internal implementation for bytecast derive macros.
//!
//! This is a regular library crate so that other proc-macro crates
//! (e.g. `pebble-macros`) can reuse the `ToBytes`/`FromBytes` code generation.

pub mod bytes;
