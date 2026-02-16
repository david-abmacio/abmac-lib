//! Tests for PebbleManager.

use alloc::vec;

use spout::DropSpout;

use crate::manager::{DirectStorage, Manifest, NoWarm, PebbleManager, PebbleManagerError};
use crate::storage::InMemoryStorage;
use crate::strategy::Strategy;

use super::fixtures::{TestCheckpoint, test_cold};

mod basic;
mod builder;
mod cold_buffer;
mod dirty;
mod rebuild;
mod recovery;
mod resize;
mod theory;
mod tombstone;

#[cfg(feature = "std")]
mod parallel;
