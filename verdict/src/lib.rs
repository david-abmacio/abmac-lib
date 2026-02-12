//! Typestate-driven error handling with compile-time status tracking.
//!
//! # Overview
//!
//! `verdict` provides a structured approach to error handling where errors carry
//! semantic information about what the caller should do (retry, give up, escalate).
//!
//! Key concepts:
//! - **Actionable errors**: Errors that declare whether they are retryable
//! - **Typestate status**: Compile-time tracking of error resolution state
//! - **Bounded context**: Configurable overflow sinks prevent unbounded allocation
//!
//! # Features
//!
//! - `std` (default): Full functionality including std::error::Error
//! - `alloc`: Heap-allocated context frames, VecDeque-based storage
//! - (none): Core types only, fully `no_std`
//!
//! # Example
//!
//! ```rust
//! use verdict::{Actionable, ErrorStatusValue};
//!
//! #[derive(Debug)]
//! struct ApiError { retryable: bool }
//!
//! impl Actionable for ApiError {
//!     fn status_value(&self) -> ErrorStatusValue {
//!         if self.retryable {
//!             ErrorStatusValue::Temporary
//!         } else {
//!             ErrorStatusValue::Permanent
//!         }
//!     }
//! }
//!
//! let err = ApiError { retryable: true };
//! assert!(err.is_retryable());
//! ```

#![no_std]
#![warn(missing_docs)]
#![warn(clippy::all)]
#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

mod actionable;
mod macros;
mod status;

#[cfg(feature = "alloc")]
mod heap;

/// Implement [`Actionable`] for an error type.
///
/// # Fixed status
///
/// For errors that are always one status:
///
/// ```
/// # use verdict::{actionable, Actionable, ErrorStatusValue};
/// #[derive(Debug)]
/// struct TimeoutError;
///
/// actionable!(TimeoutError, Temporary);
///
/// assert!(TimeoutError.is_retryable());
/// ```
///
/// ```
/// # use verdict::{actionable, Actionable, ErrorStatusValue};
/// #[derive(Debug)]
/// struct NotFoundError;
///
/// actionable!(NotFoundError, Permanent);
///
/// assert!(!NotFoundError.is_retryable());
/// ```
///
/// # Custom body
///
/// For errors that need runtime dispatch:
///
/// ```
/// # use verdict::{actionable, Actionable, ErrorStatusValue};
/// #[derive(Debug)]
/// struct ApiError { retryable: bool }
///
/// actionable!(ApiError, self => {
///     if self.retryable {
///         ErrorStatusValue::Temporary
///     } else {
///         ErrorStatusValue::Permanent
///     }
/// });
///
/// assert!(ApiError { retryable: true }.is_retryable());
/// assert!(!ApiError { retryable: false }.is_retryable());
/// ```
#[macro_export]
macro_rules! actionable {
    ($ty:ty, Temporary) => {
        impl $crate::Actionable for $ty {
            fn status_value(&self) -> $crate::ErrorStatusValue {
                $crate::ErrorStatusValue::Temporary
            }
        }
    };
    ($ty:ty, Permanent) => {
        impl $crate::Actionable for $ty {
            fn status_value(&self) -> $crate::ErrorStatusValue {
                $crate::ErrorStatusValue::Permanent
            }
        }
    };
    ($ty:ty, $self:ident => $body:expr) => {
        impl $crate::Actionable for $ty {
            fn status_value(&$self) -> $crate::ErrorStatusValue {
                $body
            }
        }
    };
}

/// Early-return with a contextualized error.
///
/// ```
/// # use verdict::{bail, actionable, Actionable, ErrorStatusValue, Contextualized};
/// #[derive(Debug)]
/// struct InvalidInput;
/// actionable!(InvalidInput, Permanent);
///
/// fn validate(x: i32) -> Result<i32, Contextualized<InvalidInput>> {
///     if x < 0 { bail!(InvalidInput); }
///     Ok(x)
/// }
///
/// assert!(validate(-1).is_err());
/// assert!(validate(1).is_ok());
/// ```
#[cfg(feature = "alloc")]
#[macro_export]
macro_rules! bail {
    ($err:expr) => {
        return ::core::result::Result::Err($crate::Contextualized::new($err))
    };
}

/// Return early with a contextualized error if a condition is not met.
///
/// ```
/// # use verdict::{ensure, actionable, Actionable, ErrorStatusValue, Contextualized};
/// #[derive(Debug)]
/// struct InvalidInput;
/// actionable!(InvalidInput, Permanent);
///
/// fn validate(x: i32) -> Result<i32, Contextualized<InvalidInput>> {
///     ensure!(x >= 0, InvalidInput);
///     Ok(x)
/// }
///
/// assert!(validate(-1).is_err());
/// assert!(validate(1).is_ok());
/// ```
#[cfg(feature = "alloc")]
#[macro_export]
macro_rules! ensure {
    ($cond:expr, $err:expr) => {
        if !$cond {
            $crate::bail!($err);
        }
    };
}

// Core exports (no_std)
pub use actionable::Actionable;
pub use status::{Dynamic, ErrorStatusValue, Permanent, Persistent, Status, Temporary};
pub use status::{NonTerminal, Terminal};

/// Common imports for typical usage.
///
/// ```
/// use verdict::prelude::*;
/// ```
pub mod prelude {
    pub use crate::{Actionable, ErrorStatusValue, actionable};

    #[cfg(feature = "alloc")]
    pub use crate::{
        ContextExt, Contextualized, Ctx, Frame, FrameRecord, LogRecord, OptionExt, ResultExt,
        RetryOutcome, bail, ensure, with_retry,
    };

    #[cfg(feature = "std")]
    pub use crate::{exponential_backoff, with_retry_delay};
}

// Alloc exports
#[cfg(feature = "alloc")]
pub use heap::*;

/// Convenience alias for the common case: a contextualized error with dynamic status.
#[cfg(feature = "alloc")]
pub type Ctx<E> = Contextualized<E, Dynamic, spout::DropSpout>;

#[cfg(test)]
mod tests;
