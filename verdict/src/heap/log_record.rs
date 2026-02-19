//! Unified logging record for structured serialization.
//!
//! `LogRecord` and `FrameRecord` are simple owned-field structs that derive
//! serialization support for all enabled frameworks (serde, facet, bytecast).
//! Convert from verdict's core types via `From` impls.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt::Display;

use spout::Spout;

use crate::{Actionable, Frame, Status};

/// A serializable snapshot of a context frame.
#[derive(Debug, Clone)]
#[allow(clippy::unsafe_derive_deserialize)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "facet", derive(facet::Facet))]
#[cfg_attr(
    feature = "bytecast",
    derive(bytecast::DeriveToBytes, bytecast::DeriveFromBytes)
)]
pub struct FrameRecord {
    /// Context message.
    pub message: String,
    /// Source file.
    pub file: String,
    /// Line number.
    pub line: u32,
    /// Column number.
    pub column: u32,
}

/// A serializable snapshot of a contextualized error for structured logging.
#[derive(Debug, Clone)]
#[allow(clippy::unsafe_derive_deserialize)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "facet", derive(facet::Facet))]
#[cfg_attr(
    feature = "bytecast",
    derive(bytecast::DeriveToBytes, bytecast::DeriveFromBytes)
)]
pub struct LogRecord {
    /// Error message (via `Display`).
    pub error: String,
    /// Status string (`"permanent"`, `"temporary"`, `"exhausted"`).
    pub status: String,
    /// Whether the error is retryable.
    pub retryable: bool,
    /// Context frames.
    pub frames: Vec<FrameRecord>,
    /// Number of frames that were evicted to overflow.
    pub overflow_count: usize,
}

impl From<&Frame> for FrameRecord {
    fn from(frame: &Frame) -> Self {
        Self {
            message: frame.msg().to_string(),
            file: frame.file().to_string(),
            line: frame.line(),
            column: frame.column(),
        }
    }
}

impl<E, S, Overflow> From<&crate::Context<E, S, Overflow>> for LogRecord
where
    E: Display + Actionable,
    S: Status,
    Overflow: Spout<Frame, Error = core::convert::Infallible>,
{
    fn from(ctx: &crate::Context<E, S, Overflow>) -> Self {
        let status = S::VALUE.unwrap_or_else(|| ctx.inner().status_value());
        Self {
            error: ctx.inner().to_string(),
            status: status.as_str().to_string(),
            retryable: status.is_retryable(),
            frames: ctx.frames().iter().map(FrameRecord::from).collect(),
            overflow_count: ctx.overflow_count(),
        }
    }
}
