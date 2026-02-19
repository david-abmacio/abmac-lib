//! Extension traits for Result types.

use alloc::string::String;

use spout::{DropSpout, Spout};

use crate::{Actionable, Context, Dynamic, Frame, Status};

/// Extension trait for `Result<T, E>` to add context.
pub trait ResultExt<T, E> {
    /// Wrap the error in [`Context`] and add context.
    ///
    /// # Errors
    ///
    /// Returns the original error wrapped in [`Context`] with the given context frame.
    fn wrap_ctx(self, message: impl Into<String>) -> Result<T, Context<E, Dynamic>>
    where
        E: Actionable;

    /// Wrap with lazy message evaluation.
    ///
    /// # Errors
    ///
    /// Returns the original error wrapped in [`Context`] with a lazily-built context frame.
    fn wrap_ctx_lazy<F: FnOnce() -> String>(self, f: F) -> Result<T, Context<E, Dynamic>>
    where
        E: Actionable;

    /// Wrap with custom overflow handling.
    ///
    /// # Errors
    ///
    /// Returns the original error wrapped in a bounded [`Context`].
    fn wrap_ctx_bounded<Overflow: Spout<Frame, Error = core::convert::Infallible>>(
        self,
        message: impl Into<String>,
        overflow: Overflow,
        max_frames: usize,
    ) -> Result<T, Context<E, Dynamic, Overflow>>
    where
        E: Actionable;
}

impl<T, E> ResultExt<T, E> for Result<T, E> {
    #[track_caller]
    fn wrap_ctx(self, message: impl Into<String>) -> Result<T, Context<E, Dynamic>>
    where
        E: Actionable,
    {
        let frame = Frame::here(message);
        self.map_err(|e| Context::new(e).with_frame(frame))
    }

    #[track_caller]
    fn wrap_ctx_lazy<F: FnOnce() -> String>(self, f: F) -> Result<T, Context<E, Dynamic>>
    where
        E: Actionable,
    {
        let location = core::panic::Location::caller();
        self.map_err(|e| {
            let frame = Frame::at(location, f());
            Context::new(e).with_frame(frame)
        })
    }

    #[track_caller]
    fn wrap_ctx_bounded<Overflow: Spout<Frame, Error = core::convert::Infallible>>(
        self,
        message: impl Into<String>,
        overflow: Overflow,
        max_frames: usize,
    ) -> Result<T, Context<E, Dynamic, Overflow>>
    where
        E: Actionable,
    {
        let frame = Frame::here(message);
        self.map_err(|e| Context::with_overflow(e, overflow, max_frames).with_frame(frame))
    }
}

/// Extension trait for `Result<T, Context<E, S, O>>` to add more context.
pub trait ContextExt<T, E, S: Status, Overflow: Spout<Frame, Error = core::convert::Infallible>> {
    /// Add context to an already-contextualized error.
    ///
    /// # Errors
    ///
    /// Passes through the original error with an additional context frame.
    fn with_ctx(self, message: impl Into<String>) -> Result<T, Context<E, S, Overflow>>;

    /// Add context with lazy evaluation.
    ///
    /// # Errors
    ///
    /// Passes through the original error with a lazily-built context frame.
    fn with_ctx_lazy<F: FnOnce() -> String>(self, f: F) -> Result<T, Context<E, S, Overflow>>;
}

impl<T, E, S: Status, Overflow: Spout<Frame, Error = core::convert::Infallible>>
    ContextExt<T, E, S, Overflow> for Result<T, Context<E, S, Overflow>>
{
    #[track_caller]
    fn with_ctx(self, message: impl Into<String>) -> Result<T, Context<E, S, Overflow>> {
        let frame = Frame::here(message);
        self.map_err(|e| e.with_frame(frame))
    }

    #[track_caller]
    fn with_ctx_lazy<F: FnOnce() -> String>(self, f: F) -> Result<T, Context<E, S, Overflow>> {
        let location = core::panic::Location::caller();
        self.map_err(|e| e.with_frame(Frame::at(location, f())))
    }
}

/// Extension trait for `Option<T>` to convert `None` into a contextualized error.
pub trait OptionExt<T> {
    /// Convert `None` into a contextualized error with a context message.
    ///
    /// # Errors
    ///
    /// Returns a [`Context`] error with the given context frame when `None`.
    fn wrap_ctx<E: Actionable>(
        self,
        error: E,
        message: impl Into<String>,
    ) -> Result<T, Context<E, Dynamic>>;

    /// Convert `None` into a contextualized error with lazy evaluation.
    ///
    /// # Errors
    ///
    /// Returns a [`Context`] error with a lazily-built context frame when `None`.
    fn wrap_ctx_lazy<E: Actionable, F: FnOnce() -> (E, String)>(
        self,
        f: F,
    ) -> Result<T, Context<E, Dynamic>>;
}

impl<T> OptionExt<T> for Option<T> {
    #[track_caller]
    fn wrap_ctx<E: Actionable>(
        self,
        error: E,
        message: impl Into<String>,
    ) -> Result<T, Context<E, Dynamic>> {
        if let Some(v) = self {
            Ok(v)
        } else {
            let frame = Frame::here(message);
            Err(Context::new(error).with_frame(frame))
        }
    }

    #[track_caller]
    fn wrap_ctx_lazy<E: Actionable, F: FnOnce() -> (E, String)>(
        self,
        f: F,
    ) -> Result<T, Context<E, Dynamic>> {
        if let Some(v) = self {
            Ok(v)
        } else {
            let location = core::panic::Location::caller();
            let (error, message) = f();
            let frame = Frame::at(location, message);
            Err(Context::new(error).with_frame(frame))
        }
    }
}

/// Extension to convert any error into a contextualized one.
pub trait IntoContext<E> {
    /// Convert into a contextualized error.
    fn into_ctx(self) -> Context<E, Dynamic, DropSpout>
    where
        E: Actionable;
}

impl<E> IntoContext<E> for E {
    fn into_ctx(self) -> Context<E, Dynamic, DropSpout>
    where
        E: Actionable,
    {
        Context::new(self)
    }
}
