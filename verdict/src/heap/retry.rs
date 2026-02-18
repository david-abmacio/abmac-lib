//! Retry helpers with typestate tracking.

use alloc::{collections::VecDeque, format};
use core::fmt::{self, Display};

use spout::DropSpout;

use crate::{Actionable, Context, Dynamic, Exhausted, Frame, Permanent, Resolved};

/// Outcome of a retry operation.
#[non_exhaustive]
pub enum RetryOutcome<E, Overflow = DropSpout>
where
    Overflow: spout::Spout<Frame, Error = core::convert::Infallible>,
{
    /// Error was permanent from the start.
    Permanent(Context<E, Permanent, Overflow>),
    /// Error was temporary but retries are exhausted.
    Exhausted(Context<E, Exhausted, Overflow>),
}

impl<E: Display, Overflow: spout::Spout<Frame, Error = core::convert::Infallible>> Display
    for RetryOutcome<E, Overflow>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Permanent(e) => write!(f, "{e}"),
            Self::Exhausted(e) => write!(f, "{e}"),
        }
    }
}

impl<E: core::fmt::Debug, Overflow: spout::Spout<Frame, Error = core::convert::Infallible>>
    core::fmt::Debug for RetryOutcome<E, Overflow>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Permanent(e) => f.debug_tuple("Permanent").field(e).finish(),
            Self::Exhausted(e) => f.debug_tuple("Exhausted").field(e).finish(),
        }
    }
}

impl<
    E: core::error::Error + 'static,
    Overflow: spout::Spout<Frame, Error = core::convert::Infallible>,
> core::error::Error for RetryOutcome<E, Overflow>
{
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Permanent(e) => e.source(),
            Self::Exhausted(e) => e.source(),
        }
    }
}

impl<E, Overflow: spout::Spout<Frame, Error = core::convert::Infallible>>
    RetryOutcome<E, Overflow>
{
    /// Check if the outcome was due to a permanent error.
    #[must_use]
    pub fn is_permanent(&self) -> bool {
        matches!(self, Self::Permanent(_))
    }

    /// Check if the outcome was due to exhausted retries.
    #[must_use]
    pub fn is_exhausted(&self) -> bool {
        matches!(self, Self::Exhausted(_))
    }

    /// Get the inner error regardless of variant.
    #[must_use]
    pub fn inner(&self) -> &E {
        match self {
            Self::Permanent(e) => e.inner(),
            Self::Exhausted(e) => e.inner(),
        }
    }

    /// Get the frames regardless of variant.
    #[must_use]
    pub fn frames(&self) -> &VecDeque<Frame> {
        match self {
            Self::Permanent(e) => e.frames(),
            Self::Exhausted(e) => e.frames(),
        }
    }
}

/// Execute an operation with retries, tracking state via typestates.
///
/// # Arguments
///
/// * `max_attempts` - Maximum number of attempts (must be > 0)
/// * `f` - The fallible operation to retry
///
/// # Errors
///
/// Returns [`RetryOutcome::Permanent`] if the operation returns a non-retryable
/// error, or [`RetryOutcome::Exhausted`] if all attempts produce retryable errors.
///
/// # Context Retention
///
/// Only the **last** temporary attempt's context (frames, backtrace, overflow)
/// is preserved. Prior attempts' context is dropped. If you need visibility
/// into all attempts, use a custom retry loop with a collecting overflow spout.
///
/// # Example
///
/// ```
/// use verdict::{with_retry, Actionable, ErrorStatusValue, Context};
///
/// #[derive(Debug)]
/// struct ApiError { retryable: bool }
///
/// impl std::fmt::Display for ApiError {
///     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
///         write!(f, "API error")
///     }
/// }
///
/// impl std::error::Error for ApiError {}
///
/// impl Actionable for ApiError {
///     fn status_value(&self) -> ErrorStatusValue {
///         if self.retryable {
///             ErrorStatusValue::Temporary
///         } else {
///             ErrorStatusValue::Permanent
///         }
///     }
/// }
///
/// let result: Result<(), _> = with_retry(3, || {
///     // Simulate an operation
///     Err(Context::new(ApiError { retryable: true })
///         .with_ctx("calling external API"))
/// });
///
/// match result {
///     Ok(value) => println!("Success: {:?}", value),
///     Err(outcome) => println!("Failed: {}", outcome),
/// }
/// ```
pub fn with_retry<T, E, F>(max_attempts: u32, mut f: F) -> Result<T, RetryOutcome<E>>
where
    E: Actionable,
    F: FnMut() -> Result<T, Context<E, Dynamic>>,
{
    let max_attempts = max_attempts.max(1);

    // Run the first attempt outside the loop to establish a non-Option binding.
    let mut last_temp = match f() {
        Ok(v) => return Ok(v),
        Err(e) => match e.resolve() {
            Resolved::Temporary(temp) => temp.with_ctx(format!("attempt 1/{max_attempts}")),
            Resolved::Exhausted(ex) => return Err(RetryOutcome::Exhausted(ex)),
            Resolved::Permanent(perm) => return Err(RetryOutcome::Permanent(perm)),
        },
    };

    for attempt in 2..=max_attempts {
        match f() {
            Ok(v) => return Ok(v),
            Err(e) => match e.resolve() {
                Resolved::Temporary(temp) => {
                    last_temp = temp.with_ctx(format!("attempt {attempt}/{max_attempts}"));
                }
                Resolved::Exhausted(ex) => return Err(RetryOutcome::Exhausted(ex)),
                Resolved::Permanent(perm) => return Err(RetryOutcome::Permanent(perm)),
            },
        }
    }

    Err(RetryOutcome::Exhausted(last_temp.exhaust()))
}

/// Execute with retries and a synchronous delay between attempts.
///
/// This function calls [`std::thread::sleep`] between attempts, which blocks
/// the current thread. Do not use inside an async runtime — use an
/// async-aware retry loop with `tokio::time::sleep` or equivalent instead.
///
/// # Errors
///
/// Returns [`RetryOutcome::Permanent`] if the operation returns a non-retryable
/// error, or [`RetryOutcome::Exhausted`] if all attempts produce retryable errors.
#[cfg(feature = "std")]
pub fn with_retry_delay<T, E, F, D>(
    max_attempts: u32,
    mut delay: D,
    mut f: F,
) -> Result<T, RetryOutcome<E>>
where
    E: Actionable,
    F: FnMut() -> Result<T, Context<E, Dynamic>>,
    D: FnMut(u32) -> std::time::Duration,
{
    let max_attempts = max_attempts.max(1);

    // Run the first attempt outside the loop to establish a non-Option binding.
    let mut last_temp = match f() {
        Ok(v) => return Ok(v),
        Err(e) => match e.resolve() {
            Resolved::Temporary(temp) => temp.with_ctx(format!("attempt 1/{max_attempts}")),
            Resolved::Exhausted(ex) => return Err(RetryOutcome::Exhausted(ex)),
            Resolved::Permanent(perm) => return Err(RetryOutcome::Permanent(perm)),
        },
    };

    for attempt in 2..=max_attempts {
        std::thread::sleep(delay(attempt - 1));

        match f() {
            Ok(v) => return Ok(v),
            Err(e) => match e.resolve() {
                Resolved::Temporary(temp) => {
                    last_temp = temp.with_ctx(format!("attempt {attempt}/{max_attempts}"));
                }
                Resolved::Exhausted(ex) => return Err(RetryOutcome::Exhausted(ex)),
                Resolved::Permanent(perm) => return Err(RetryOutcome::Permanent(perm)),
            },
        }
    }

    Err(RetryOutcome::Exhausted(last_temp.exhaust()))
}

/// Exponential backoff delay calculator.
///
/// Returns `base * 2^(attempt-1)` capped at `max`. An `attempt` of 0 is
/// treated as 1 (no scaling).
#[cfg(feature = "std")]
pub fn exponential_backoff(
    base: std::time::Duration,
    max: std::time::Duration,
) -> impl FnMut(u32) -> std::time::Duration {
    move |attempt| {
        let shift = attempt.saturating_sub(1).min(31);
        let multiplier = 1u32 << shift;
        let delay = base.saturating_mul(multiplier);
        delay.min(max)
    }
}
