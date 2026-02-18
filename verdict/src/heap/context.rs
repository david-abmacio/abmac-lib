//! The Context error wrapper with typestate status tracking.

use alloc::{collections::VecDeque, string::String};
use core::fmt::{self, Debug, Display};
use core::marker::PhantomData;

use spout::{DropSpout, Spout};

use crate::{
    Actionable, Dynamic, ErrorStatusValue, Exhausted, Frame, Permanent, Status, Temporary,
};

/// Error wrapper with typestate status and context frames.
///
/// # Type Parameters
///
/// - `E`: The underlying error type (must implement `Actionable`)
/// - `S`: Status typestate (`Dynamic`, `Temporary`, `Exhausted`, `Permanent`)
/// - `Overflow`: [`Spout`] for evicted frames when at capacity (default: [`DropSpout`])
///
/// # Overflow Handling
///
/// By default, context frames are unbounded (stored in a `VecDeque`). For memory-sensitive
/// applications, you can limit frames and handle overflow:
///
/// ```rust
/// use verdict::{Actionable, ErrorStatusValue, Context, CollectSpout};
///
/// #[derive(Debug)]
/// struct MyError;
/// impl std::fmt::Display for MyError {
///     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
///         write!(f, "my error")
///     }
/// }
/// impl std::error::Error for MyError {}
/// impl Actionable for MyError {
///     fn status_value(&self) -> ErrorStatusValue { ErrorStatusValue::Temporary }
/// }
///
/// // Keep last 8 frames, collect overflow
/// let overflow = CollectSpout::new();
/// let err = Context::with_overflow(MyError, overflow, 8);
/// ```
///
/// # State Machine
///
/// ```text
///                    refine()
///     ┌─────────────────┼─────────────────┐
///     │                 │                 │
///     ▼                 │                 ▼
/// ┌───────┐     Ok  ┌───┴─────┐  Err  ┌─────────┐
/// │Dynamic├─────────►Temporary├───────►Permanent│
/// └───────┘         └────┬────┘       └─────────┘
///                        │
///                        │ exhaust()
///                        ▼
///                  ┌──────────┐
///                  │Exhausted│
///                  └──────────┘
/// ```
pub struct Context<
    E,
    S: Status = Dynamic,
    Overflow: Spout<Frame, Error = core::convert::Infallible> = DropSpout,
> {
    error: E,
    frames: VecDeque<Frame>,
    overflow: Overflow,
    max_frames: usize,
    overflow_count: usize,
    #[cfg(feature = "std")]
    backtrace: std::backtrace::Backtrace,
    _status: PhantomData<S>,
}

// Constructors

impl<E: Actionable> Context<E, Dynamic, DropSpout> {
    /// Create a new contextualized error with unbounded frames.
    ///
    /// When the `std` feature is enabled, a backtrace is captured automatically.
    /// Capture is controlled by the `RUST_BACKTRACE` environment variable and
    /// is a no-op when unset — there is no per-call opt-out.
    #[must_use]
    pub fn new(error: E) -> Self {
        Self {
            error,
            frames: VecDeque::new(),
            overflow: DropSpout,
            max_frames: usize::MAX,
            overflow_count: 0,
            #[cfg(feature = "std")]
            backtrace: std::backtrace::Backtrace::capture(),
            _status: PhantomData,
        }
    }

    /// Create a bounded contextualized error that drops evicted frames.
    ///
    /// When more than `max_frames` are added, the oldest frames are silently
    /// discarded. A `max_frames` of 0 is clamped to 1.
    #[must_use]
    pub fn bounded(error: E, max_frames: usize) -> Self {
        let max_frames = max_frames.max(1);
        Self {
            error,
            frames: VecDeque::with_capacity(max_frames),
            overflow: DropSpout,
            max_frames,
            overflow_count: 0,
            #[cfg(feature = "std")]
            backtrace: std::backtrace::Backtrace::capture(),
            _status: PhantomData,
        }
    }
}

impl<E: Actionable> Context<E, Dynamic, spout::CollectSpout<Frame>> {
    /// Create a bounded contextualized error that collects evicted frames.
    ///
    /// When more than `max_frames` are added, the oldest frames are moved to
    /// an internal collection accessible via [`into_overflow`](Context::into_overflow).
    /// A `max_frames` of 0 is clamped to 1.
    ///
    /// **Note:** The overflow collection is unbounded — total memory usage is
    /// not reduced, only the active frame window is limited. For true memory
    /// bounding, use [`Context::bounded`] which drops evicted frames.
    #[must_use]
    pub fn bounded_collect(error: E, max_frames: usize) -> Self {
        let max_frames = max_frames.max(1);
        Self {
            error,
            frames: VecDeque::with_capacity(max_frames),
            overflow: spout::CollectSpout::new(),
            max_frames,
            overflow_count: 0,
            #[cfg(feature = "std")]
            backtrace: std::backtrace::Backtrace::capture(),
            _status: PhantomData,
        }
    }
}

impl<E: Actionable, Overflow: Spout<Frame, Error = core::convert::Infallible>>
    Context<E, Dynamic, Overflow>
{
    /// Create with custom overflow handling.
    ///
    /// When frames exceed `max_frames`, the oldest frame is evicted to the
    /// overflow spout before adding new context.
    ///
    /// A `max_frames` of 0 is clamped to 1.
    #[must_use]
    pub fn with_overflow(error: E, overflow: Overflow, max_frames: usize) -> Self {
        let max_frames = max_frames.max(1);
        Self {
            error,
            frames: VecDeque::with_capacity(max_frames),
            overflow,
            max_frames,
            overflow_count: 0,
            #[cfg(feature = "std")]
            backtrace: std::backtrace::Backtrace::capture(),
            _status: PhantomData,
        }
    }
}

// Common Methods (all states)

impl<E, S: Status, Overflow: Spout<Frame, Error = core::convert::Infallible>>
    Context<E, S, Overflow>
{
    /// Get a reference to the underlying error.
    #[must_use]
    pub fn inner(&self) -> &E {
        &self.error
    }

    /// Consume and return the underlying error.
    #[must_use]
    pub fn into_inner(self) -> E {
        self.error
    }

    /// Get the context frames (most recent last).
    #[must_use]
    pub fn frames(&self) -> &VecDeque<Frame> {
        &self.frames
    }

    /// Get the number of frames that overflowed.
    #[must_use]
    pub fn overflow_count(&self) -> usize {
        self.overflow_count
    }

    /// Check if any frames were evicted to overflow.
    #[must_use]
    pub fn has_overflow(&self) -> bool {
        self.overflow_count > 0
    }

    /// Access the overflow spout.
    #[must_use]
    pub fn overflow(&self) -> &Overflow {
        &self.overflow
    }

    /// Mutably access the overflow spout.
    #[must_use]
    pub fn overflow_mut(&mut self) -> &mut Overflow {
        &mut self.overflow
    }

    /// Consume and return the overflow spout.
    #[must_use]
    pub fn into_overflow(self) -> Overflow {
        self.overflow
    }

    /// Get the captured backtrace from when this error was created.
    #[cfg(feature = "std")]
    pub fn backtrace(&self) -> &std::backtrace::Backtrace {
        &self.backtrace
    }

    /// Add context at the caller's location.
    #[must_use]
    #[track_caller]
    pub fn with_ctx(mut self, message: impl Into<String>) -> Self {
        self.add_context(Frame::here(message));
        self
    }

    /// Add context with lazy message evaluation.
    #[must_use]
    #[track_caller]
    pub fn with_ctx_lazy<F: FnOnce() -> String>(mut self, f: F) -> Self {
        self.add_context(Frame::here(f()));
        self
    }

    /// Add a pre-built frame.
    #[must_use]
    pub fn with_frame(mut self, frame: Frame) -> Self {
        self.add_context(frame);
        self
    }

    fn add_context(&mut self, frame: Frame) {
        if self.frames.len() >= self.max_frames {
            // Evict oldest frame to overflow
            if let Some(evicted) = self.frames.pop_front() {
                let _ = self.overflow.send(evicted);
                self.overflow_count = self.overflow_count.saturating_add(1);
            }
        }
        self.frames.push_back(frame);
    }

    /// Transform to a different status (internal).
    fn transition<S2: Status>(self) -> Context<E, S2, Overflow> {
        Context {
            error: self.error,
            frames: self.frames,
            overflow: self.overflow,
            max_frames: self.max_frames,
            overflow_count: self.overflow_count,
            #[cfg(feature = "std")]
            backtrace: self.backtrace,
            _status: PhantomData,
        }
    }

    // Debug Assertions

    /// Assert minimum context depth (debug builds only).
    #[must_use]
    #[track_caller]
    pub fn assert_depth(self, min: usize) -> Self {
        debug_assert!(
            self.frames.len() >= min,
            "insufficient context: expected >= {}, got {}",
            min,
            self.frames.len()
        );
        self
    }

    /// Assert that context includes a frame from a specific module (debug builds only).
    #[must_use]
    #[track_caller]
    pub fn assert_origin(self, module_prefix: &str) -> Self {
        debug_assert!(
            self.frames.iter().any(|f| f.file().contains(module_prefix)),
            "missing provenance: expected frame from '{}'",
            module_prefix
        );
        self
    }
}

// Dynamic State

/// Result of resolving a [`Context<E, Dynamic>`] into a concrete status.
///
/// Returned by [`Context::resolve()`]. Each variant carries the context
/// transitioned to the matching typestate.
pub enum Resolved<E, Overflow: Spout<Frame, Error = core::convert::Infallible> = DropSpout> {
    /// Error is temporary and may succeed on retry.
    Temporary(Context<E, Temporary, Overflow>),
    /// Error was temporary but retries are exhausted.
    Exhausted(Context<E, Exhausted, Overflow>),
    /// Error is permanent and should not be retried.
    Permanent(Context<E, Permanent, Overflow>),
}

impl<E: Debug, Overflow: Spout<Frame, Error = core::convert::Infallible>> Debug
    for Resolved<E, Overflow>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Temporary(c) => f.debug_tuple("Temporary").field(c).finish(),
            Self::Exhausted(c) => f.debug_tuple("Exhausted").field(c).finish(),
            Self::Permanent(c) => f.debug_tuple("Permanent").field(c).finish(),
        }
    }
}

impl<E: Actionable, Overflow: Spout<Frame, Error = core::convert::Infallible>>
    Context<E, Dynamic, Overflow>
{
    /// Refine to a concrete status based on the error's status value.
    pub fn resolve(self) -> Resolved<E, Overflow> {
        match self.error.status_value() {
            ErrorStatusValue::Temporary => Resolved::Temporary(self.transition()),
            ErrorStatusValue::Exhausted => Resolved::Exhausted(self.transition()),
            ErrorStatusValue::Permanent => Resolved::Permanent(self.transition()),
        }
    }

    /// Check if the underlying error is retryable.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        self.error.status_value() == ErrorStatusValue::Temporary
    }

    /// Get the runtime status value.
    #[must_use]
    pub fn status_value(&self) -> ErrorStatusValue {
        self.error.status_value()
    }
}

// Temporary State

impl<E, Overflow: Spout<Frame, Error = core::convert::Infallible>> Context<E, Temporary, Overflow> {
    /// Mark retries as exhausted, transitioning to `Exhausted`.
    #[must_use]
    pub fn exhaust(self) -> Context<E, Exhausted, Overflow> {
        self.transition()
    }

    /// Compile-time proof that this error is retryable.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        true
    }
}

// Terminal States

impl<E, Overflow: Spout<Frame, Error = core::convert::Infallible>> Context<E, Exhausted, Overflow> {
    /// Compile-time proof that this error is not retryable.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        false
    }
}

impl<E, Overflow: Spout<Frame, Error = core::convert::Infallible>> Context<E, Permanent, Overflow> {
    /// Compile-time proof that this error is not retryable.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        false
    }
}

// Trait Implementations

impl<E: Actionable, S: Status, Overflow: Spout<Frame, Error = core::convert::Infallible>> Actionable
    for Context<E, S, Overflow>
{
    fn status_value(&self) -> ErrorStatusValue {
        // Use compile-time status if available, otherwise delegate
        S::VALUE.unwrap_or_else(|| self.error.status_value())
    }
}

impl<E: Display, S: Status, Overflow: Spout<Frame, Error = core::convert::Infallible>> Display
    for Context<E, S, Overflow>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.error)?;
        for frame in &self.frames {
            write!(f, "\n  |-> {frame}")?;
        }
        if self.overflow_count > 0 {
            write!(
                f,
                "\n  |-> ... ({} earlier frames omitted)",
                self.overflow_count
            )?;
        }
        #[cfg(feature = "std")]
        if self.backtrace.status() == std::backtrace::BacktraceStatus::Captured {
            write!(f, "\n\n{}", self.backtrace)?;
        }
        Ok(())
    }
}

impl<E: Debug, S: Status, Overflow: Spout<Frame, Error = core::convert::Infallible>> Debug
    for Context<E, S, Overflow>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = f.debug_struct("Context");
        s.field("error", &self.error)
            .field("frames", &self.frames)
            .field("overflow_count", &self.overflow_count)
            .field("max_frames", &self.max_frames)
            .field("status", &S::name());
        #[cfg(feature = "std")]
        s.field("backtrace", &self.backtrace);
        s.finish_non_exhaustive()
    }
}

impl<E: Actionable> From<E> for Context<E, Dynamic, DropSpout> {
    fn from(error: E) -> Self {
        Self::new(error)
    }
}

impl<
    E: core::error::Error + 'static,
    S: Status,
    Overflow: Spout<Frame, Error = core::convert::Infallible>,
> core::error::Error for Context<E, S, Overflow>
{
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&self.error)
    }
}

// Bytecast serialization support

#[cfg(feature = "bytecast")]
impl<
    E: bytecast::ToBytes + Actionable,
    S: Status,
    Overflow: Spout<Frame, Error = core::convert::Infallible>,
> bytecast::ToBytes for Context<E, S, Overflow>
{
    fn to_bytes(&self, buf: &mut [u8]) -> Result<usize, bytecast::BytesError> {
        let status = S::VALUE.unwrap_or_else(|| self.error.status_value());
        let mut offset = 0;
        offset += status.to_bytes(&mut buf[offset..])?;
        offset += self.error.to_bytes(&mut buf[offset..])?;
        offset += self.frames.to_bytes(&mut buf[offset..])?;
        offset += self.max_frames.to_bytes(&mut buf[offset..])?;
        offset += self.overflow_count.to_bytes(&mut buf[offset..])?;
        Ok(offset)
    }

    fn byte_len(&self) -> Option<usize> {
        Some(
            ErrorStatusValue::MAX_SIZE?
                + self.error.byte_len()?
                + self.frames.byte_len()?
                + self.max_frames.byte_len()?
                + self.overflow_count.byte_len()?,
        )
    }
}

/// Always returns `Context<E, Dynamic>` — the on-wire status is discarded.
/// Use [`decode_context`] to restore the original status typestate.
#[cfg(feature = "bytecast")]
impl<E: bytecast::FromBytes + Actionable> bytecast::FromBytes for Context<E, Dynamic, DropSpout> {
    fn from_bytes(buf: &[u8]) -> Result<(Self, usize), bytecast::BytesError> {
        let mut offset = 0;
        let (_status, n) = ErrorStatusValue::from_bytes(&buf[offset..])?;
        offset += n;
        let (error, n) = E::from_bytes(&buf[offset..])?;
        offset += n;
        let (frames, n) = VecDeque::<Frame>::from_bytes(&buf[offset..])?;
        offset += n;
        let (max_frames, n) = usize::from_bytes(&buf[offset..])?;
        offset += n;
        let max_frames = max_frames.max(1);
        let (overflow_count, n) = usize::from_bytes(&buf[offset..])?;
        offset += n;

        Ok((
            Self {
                error,
                frames,
                overflow: DropSpout,
                max_frames,
                overflow_count,
                #[cfg(feature = "std")]
                backtrace: std::backtrace::Backtrace::disabled(),
                _status: PhantomData,
            },
            offset,
        ))
    }
}

/// Decoded context with the correct typestate from serialized bytes.
///
/// There is no `Dynamic` variant because the wire format always carries a
/// concrete status value — `Dynamic` only exists at the type level before
/// [`Context::resolve()`] is called.
#[cfg(feature = "bytecast")]
pub enum DecodedContext<E> {
    /// Status was `Temporary` (retryable).
    Temporary(Context<E, Temporary, DropSpout>),
    /// Status was `Exhausted` (retries exhausted).
    Exhausted(Context<E, Exhausted, DropSpout>),
    /// Status was `Permanent` (never retryable).
    Permanent(Context<E, Permanent, DropSpout>),
}

/// Decode a serialized [`Context`] with the correct typestate.
///
/// Unlike `FromBytes for Context<E, Dynamic, DropSpout>` which always returns
/// `Dynamic`, this function restores the status typestate that was present
/// when the context was serialized.
#[cfg(feature = "bytecast")]
pub fn decode_context<E: bytecast::FromBytes + Actionable>(
    buf: &[u8],
) -> Result<(DecodedContext<E>, usize), bytecast::BytesError> {
    use bytecast::FromBytes;

    let mut offset = 0;
    let (status, n) = ErrorStatusValue::from_bytes(&buf[offset..])?;
    offset += n;
    let (error, n) = E::from_bytes(&buf[offset..])?;
    offset += n;
    let (frames, n) = VecDeque::<Frame>::from_bytes(&buf[offset..])?;
    offset += n;
    let (max_frames, n) = usize::from_bytes(&buf[offset..])?;
    offset += n;
    let max_frames = max_frames.max(1);
    let (overflow_count, n) = usize::from_bytes(&buf[offset..])?;
    offset += n;

    macro_rules! build {
        ($status_type:ty, $variant:ident) => {{
            let ctx = Context {
                error,
                frames,
                overflow: DropSpout,
                max_frames,
                overflow_count,
                #[cfg(feature = "std")]
                backtrace: std::backtrace::Backtrace::disabled(),
                _status: PhantomData::<$status_type>,
            };
            Ok((DecodedContext::$variant(ctx), offset))
        }};
    }

    match status {
        ErrorStatusValue::Temporary => build!(Temporary, Temporary),
        ErrorStatusValue::Exhausted => build!(Exhausted, Exhausted),
        ErrorStatusValue::Permanent => build!(Permanent, Permanent),
    }
}
