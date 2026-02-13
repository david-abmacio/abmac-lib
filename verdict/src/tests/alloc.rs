//! Tests requiring the `alloc` feature.

use alloc::format;
use spout::{CollectSpout, Spout};

use crate::*;

#[derive(Debug, Clone)]
struct TestError {
    status: ErrorStatusValue,
    message: &'static str,
}

impl TestError {
    fn temporary(message: &'static str) -> Self {
        Self {
            status: ErrorStatusValue::Temporary,
            message,
        }
    }

    fn permanent(message: &'static str) -> Self {
        Self {
            status: ErrorStatusValue::Permanent,
            message,
        }
    }
}

impl core::fmt::Display for TestError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl core::error::Error for TestError {}

impl Actionable for TestError {
    fn status_value(&self) -> ErrorStatusValue {
        self.status
    }
}

// Context Tests

#[test]
fn test_contextualized_new() {
    let err = TestError::temporary("test error");
    let ctx = Context::new(err);

    assert_eq!(ctx.inner().message, "test error");
    assert!(ctx.frames().is_empty());
    assert!(!ctx.has_overflow());
}

#[test]
fn test_contextualized_with_ctx() {
    let err = TestError::temporary("test error");
    let ctx = Context::new(err).with_ctx("layer 1").with_ctx("layer 2");

    assert_eq!(ctx.frames().len(), 2);
    assert!(ctx.frames()[0].msg().contains("layer 1"));
    assert!(ctx.frames()[1].msg().contains("layer 2"));
}

#[test]
fn test_contextualized_refine_temporary() {
    let err = TestError::temporary("test error");
    let ctx = Context::new(err);

    match ctx.resolve() {
        Resolved::Temporary(temp) => assert!(temp.is_retryable()),
        other => panic!("expected Temporary, got {other:?}"),
    }
}

#[test]
fn test_contextualized_refine_permanent() {
    let err = TestError::permanent("test error");
    let ctx = Context::new(err);

    match ctx.resolve() {
        Resolved::Permanent(perm) => assert!(!perm.is_retryable()),
        other => panic!("expected Permanent, got {other:?}"),
    }
}

#[test]
fn test_contextualized_exhaust() {
    let err = TestError::temporary("test error");
    let ctx = Context::new(err);

    let Resolved::Temporary(temp) = ctx.resolve() else {
        panic!("expected Temporary");
    };
    let exhausted = temp.exhaust();

    assert!(!exhausted.is_retryable());
}

#[test]
fn test_contextualized_overflow() {
    let err = TestError::temporary("test error");
    let overflow = CollectSpout::new();
    let mut ctx = Context::with_overflow(err, overflow, 2);

    ctx = ctx
        .with_ctx("frame 1")
        .with_ctx("frame 2")
        .with_ctx("frame 3")
        .with_ctx("frame 4");

    // Only last 2 frames kept
    assert_eq!(ctx.frames().len(), 2);
    assert!(ctx.frames()[0].msg().contains("frame 3"));
    assert!(ctx.frames()[1].msg().contains("frame 4"));

    // 2 frames overflowed
    assert_eq!(ctx.overflow_count(), 2);
    assert!(ctx.has_overflow());

    // Check overflow sink
    let collected = ctx.into_overflow().into_items();
    assert_eq!(collected.len(), 2);
    assert!(collected[0].msg().contains("frame 1"));
    assert!(collected[1].msg().contains("frame 2"));
}

#[test]
fn test_contextualized_display() {
    let err = TestError::temporary("root cause");
    let ctx = Context::new(err)
        .with_ctx("doing thing A")
        .with_ctx("doing thing B");

    let display = format!("{ctx}");

    assert!(display.contains("root cause"));
    assert!(display.contains("doing thing A"));
    assert!(display.contains("doing thing B"));
}

#[test]
fn test_contextualized_display_with_overflow() {
    let err = TestError::temporary("root cause");
    let ctx = Context::with_overflow(err, DropSpout, 1)
        .with_ctx("frame 1")
        .with_ctx("frame 2");

    let display = format!("{ctx}");

    assert!(display.contains("root cause"));
    assert!(display.contains("frame 2"));
    assert!(display.contains("1 earlier frames omitted"));
}

// Result Extension Tests

#[test]
fn test_result_ext_wrap_ctx() {
    let result: Result<(), TestError> = Err(TestError::temporary("test"));
    let ctx = result.wrap_ctx("adding context");

    assert!(ctx.is_err());
    let err = ctx.unwrap_err();
    assert_eq!(err.frames().len(), 1);
}

#[test]
fn test_result_ext_with_ctx() {
    let result: Result<(), Context<TestError>> =
        Err(Context::new(TestError::temporary("test")).with_ctx("ctx1"));

    let ctx = result.with_ctx("ctx2");

    assert!(ctx.is_err());
    let err = ctx.unwrap_err();
    assert_eq!(err.frames().len(), 2);
}

// Retry Tests

#[test]
fn test_retry_success_first_try() {
    let mut attempts = 0;

    let result = with_retry(3, || {
        attempts += 1;
        Ok::<_, Context<TestError>>(42)
    });

    assert_eq!(result.unwrap(), 42);
    assert_eq!(attempts, 1);
}

#[test]
fn test_retry_success_after_failures() {
    let mut attempts = 0;

    let result = with_retry(3, || {
        attempts += 1;
        if attempts < 3 {
            Err(Context::new(TestError::temporary("try again")))
        } else {
            Ok(42)
        }
    });

    assert_eq!(result.unwrap(), 42);
    assert_eq!(attempts, 3);
}

#[test]
fn test_retry_exhausted() {
    let mut attempts = 0;

    let result: Result<(), _> = with_retry(3, || {
        attempts += 1;
        Err(Context::new(TestError::temporary("always fails")))
    });

    assert!(result.is_err());
    let outcome = result.unwrap_err();
    assert!(outcome.is_exhausted());
    assert_eq!(attempts, 3);
}

#[test]
fn test_retry_permanent_stops_immediately() {
    let mut attempts = 0;

    let result: Result<(), _> = with_retry(3, || {
        attempts += 1;
        Err(Context::new(TestError::permanent("permanent error")))
    });

    assert!(result.is_err());
    let outcome = result.unwrap_err();
    assert!(outcome.is_permanent());
    assert_eq!(attempts, 1);
}

// Spout Tests

#[test]
fn test_frame_formatter() {
    let mut formatter = FrameFormatter::new();

    let _ = formatter.send(Frame::message("frame 1"));
    let _ = formatter.send(Frame::message("frame 2"));

    assert_eq!(formatter.count(), 2);
    assert!(formatter.as_str().contains("frame 1"));
    assert!(formatter.as_str().contains("frame 2"));
}

#[test]
fn test_counting_spout() {
    let counter = CountingSpout::new();
    let mut spout = &counter;

    let _ = spout.send(Frame::message("a"));
    let _ = spout.send(Frame::message("b"));
    let _ = spout.send(Frame::message("c"));

    assert_eq!(counter.count(), 3);

    counter.reset();
    assert_eq!(counter.count(), 0);
}

#[test]
fn test_tee_spout() {
    let collect = CollectSpout::new();
    let counter = CountingSpout::new();
    let mut tee = TeeSpout::new(collect, counter);

    let _ = tee.send(Frame::message("test"));

    let (collected, counter) = tee.into_inner();
    assert_eq!(collected.into_items().len(), 1);
    assert_eq!(counter.count(), 1);
}

// resolve() Exhausted branch

#[test]
fn test_contextualized_resolve_exhausted() {
    let err = TestError {
        status: ErrorStatusValue::Exhausted,
        message: "exhausted error",
    };
    let ctx = Context::new(err);

    match ctx.resolve() {
        Resolved::Exhausted(ex) => {
            assert!(!ex.is_retryable());
            assert_eq!(ex.inner().message, "exhausted error");
        }
        other => panic!("expected Exhausted, got {other:?}"),
    }
}

// Context accessor tests

#[test]
fn test_contextualized_into_inner() {
    let err = TestError::temporary("inner test");
    let ctx = Context::new(err);
    let inner = ctx.into_inner();
    assert_eq!(inner.message, "inner test");
}

#[test]
fn test_contextualized_with_frame() {
    let frame = Frame::new("custom.rs", 99, 1, "custom frame");
    let ctx = Context::new(TestError::temporary("test")).with_frame(frame);

    assert_eq!(ctx.frames().len(), 1);
    assert_eq!(ctx.frames()[0].file(), "custom.rs");
    assert_eq!(ctx.frames()[0].line(), 99);
    assert_eq!(ctx.frames()[0].msg(), "custom frame");
}

#[test]
fn test_contextualized_with_ctx_lazy() {
    let ctx = Context::new(TestError::temporary("test"))
        .with_ctx_lazy(|| alloc::format!("lazy {}", "context"));

    assert_eq!(ctx.frames().len(), 1);
    assert!(ctx.frames()[0].msg().contains("lazy context"));
}

#[test]
fn test_contextualized_error_source() {
    let err = TestError::temporary("root");
    let ctx = Context::new(err);
    let source = core::error::Error::source(&ctx);
    assert!(source.is_some());
    assert_eq!(format!("{}", source.unwrap()), "root");
}

#[test]
fn test_contextualized_from_error() {
    let err = TestError::temporary("from test");
    let ctx: Context<TestError> = err.into();
    assert_eq!(ctx.inner().message, "from test");
    assert!(ctx.frames().is_empty());
}

// Actionable Implementation Tests

#[test]
fn test_contextualized_actionable() {
    let err = TestError::temporary("test");
    let ctx = Context::new(err);

    assert_eq!(ctx.status_value(), ErrorStatusValue::Temporary);
}

#[test]
fn test_refined_actionable_uses_compile_time_status() {
    let err = TestError::temporary("test");
    let ctx = Context::new(err);
    let Resolved::Temporary(temp) = ctx.resolve() else {
        panic!("expected Temporary");
    };

    // Even if inner error somehow changed, the typestate controls status
    assert_eq!(temp.status_value(), ErrorStatusValue::Temporary);
}

// Frame Constructor and Accessor Tests

#[test]
fn test_frame_here() {
    let frame = Frame::here("test message");
    assert!(frame.file().contains("alloc.rs"));
    assert!(frame.line() > 0);
    assert!(frame.column() > 0);
    assert_eq!(frame.msg(), "test message");
}

#[test]
fn test_frame_at() {
    let location = core::panic::Location::caller();
    let frame = Frame::at(location, "at message");
    assert_eq!(frame.file(), location.file());
    assert_eq!(frame.line(), location.line());
    assert_eq!(frame.column(), location.column());
    assert_eq!(frame.msg(), "at message");
}

#[test]
fn test_frame_new() {
    let frame = Frame::new("src/main.rs", 42, 5, "explicit location");
    assert_eq!(frame.file(), "src/main.rs");
    assert_eq!(frame.line(), 42);
    assert_eq!(frame.column(), 5);
    assert_eq!(frame.msg(), "explicit location");
}

#[test]
fn test_frame_message() {
    let frame = Frame::message("unknown location");
    assert_eq!(frame.file(), "<unknown>");
    assert_eq!(frame.line(), 0);
    assert_eq!(frame.column(), 0);
    assert_eq!(frame.msg(), "unknown location");
}

#[test]
fn test_frame_display_with_location() {
    let frame = Frame::new("src/lib.rs", 10, 3, "something happened");
    let display = format!("{frame}");
    assert!(display.contains("something happened"));
    assert!(display.contains("src/lib.rs:10:3"));
}

#[test]
fn test_frame_display_unknown_location() {
    let frame = Frame::message("bare message");
    let display = format!("{frame}");
    assert_eq!(display, "bare message");
    assert!(!display.contains("<unknown>"));
}

#[test]
fn test_frame_equality() {
    let a = Frame::new("file.rs", 1, 1, "msg");
    let b = Frame::new("file.rs", 1, 1, "msg");
    let c = Frame::new("file.rs", 1, 1, "different");
    assert_eq!(a, b);
    assert_ne!(a, c);
}

// Bounded Constructor Tests

#[test]
fn test_contextualized_bounded() {
    let err = TestError::temporary("bounded test");
    let ctx = Context::bounded(err, 2)
        .with_ctx("frame 1")
        .with_ctx("frame 2")
        .with_ctx("frame 3");

    assert_eq!(ctx.frames().len(), 2);
    assert_eq!(ctx.overflow_count(), 1);
    assert!(ctx.frames()[0].msg().contains("frame 2"));
    assert!(ctx.frames()[1].msg().contains("frame 3"));
}

#[test]
fn test_contextualized_bounded_collect() {
    let err = TestError::temporary("bounded collect test");
    let ctx = Context::bounded_collect(err, 2)
        .with_ctx("frame 1")
        .with_ctx("frame 2")
        .with_ctx("frame 3");

    assert_eq!(ctx.frames().len(), 2);
    assert_eq!(ctx.overflow_count(), 1);

    let collected = ctx.into_overflow().into_items();
    assert_eq!(collected.len(), 1);
    assert!(collected[0].msg().contains("frame 1"));
}

#[test]
fn test_contextualized_bounded_zero_clamps_to_one() {
    let err = TestError::temporary("clamp test");
    let ctx = Context::bounded(err, 0)
        .with_ctx("frame 1")
        .with_ctx("frame 2");

    // max_frames of 0 should clamp to 1
    assert_eq!(ctx.frames().len(), 1);
    assert!(ctx.frames()[0].msg().contains("frame 2"));
}

// Lazy Context Tests

#[test]
fn test_wrap_ctx_lazy() {
    let result: Result<(), TestError> = Err(TestError::temporary("test"));
    let ctx = result.wrap_ctx_lazy(|| format!("lazy {}", "message"));

    let err = ctx.unwrap_err();
    assert_eq!(err.frames().len(), 1);
    assert!(err.frames()[0].msg().contains("lazy message"));
}

#[test]
fn test_wrap_ctx_lazy_not_called_on_ok() {
    let mut called = false;
    let result: Result<i32, TestError> = Ok(42);
    let ok = result.wrap_ctx_lazy(|| {
        called = true;
        "should not run".into()
    });

    assert_eq!(ok.unwrap(), 42);
    assert!(!called);
}

#[test]
fn test_with_ctx_lazy() {
    let result: Result<(), Context<TestError>> = Err(Context::new(TestError::temporary("test")));
    let ctx = result.with_ctx_lazy(|| format!("lazy {}", "context"));

    let err = ctx.unwrap_err();
    assert_eq!(err.frames().len(), 1);
    assert!(err.frames()[0].msg().contains("lazy context"));
}

#[test]
fn test_with_ctx_lazy_not_called_on_ok() {
    let mut called = false;
    let result: Result<i32, Context<TestError>> = Ok(42);
    let ok = result.with_ctx_lazy(|| {
        called = true;
        "should not run".into()
    });

    assert_eq!(ok.unwrap(), 42);
    assert!(!called);
}

// wrap_ctx_bounded Tests

#[test]
fn test_wrap_ctx_bounded() {
    let result: Result<(), TestError> = Err(TestError::temporary("test"));
    let ctx = result.wrap_ctx_bounded("bounded context", CollectSpout::new(), 2);

    let mut err = ctx.unwrap_err();
    assert_eq!(err.frames().len(), 1);
    assert!(err.frames()[0].msg().contains("bounded context"));

    // Add frames beyond the limit to verify eviction
    err = err.with_ctx("frame 2").with_ctx("frame 3");
    assert_eq!(err.frames().len(), 2);
    assert_eq!(err.overflow_count(), 1);
    assert!(err.frames()[0].msg().contains("frame 2"));
    assert!(err.frames()[1].msg().contains("frame 3"));

    // Evicted frame landed in the CollectSpout
    let collected = err.into_overflow().into_items();
    assert_eq!(collected.len(), 1);
    assert!(collected[0].msg().contains("bounded context"));
}

// IntoContext Tests

#[test]
fn test_into_ctx() {
    let err = TestError::temporary("into test");
    let ctx: Context<TestError> = err.into_ctx();

    assert_eq!(ctx.inner().message, "into test");
    assert!(ctx.frames().is_empty());
}

// FrameFormatter Extended Tests

#[test]
fn test_frame_formatter_with_capacity() {
    let formatter = FrameFormatter::with_capacity(1024);
    assert_eq!(formatter.count(), 0);
    assert!(formatter.as_str().is_empty());
}

#[test]
fn test_frame_formatter_into_string() {
    let mut formatter = FrameFormatter::new();
    let _ = formatter.send(Frame::message("test frame"));

    let s = formatter.into_string();
    assert!(s.contains("test frame"));
}

#[test]
fn test_frame_formatter_clear() {
    let mut formatter = FrameFormatter::new();
    let _ = formatter.send(Frame::message("frame 1"));
    let _ = formatter.send(Frame::message("frame 2"));

    assert_eq!(formatter.count(), 2);
    assert!(!formatter.as_str().is_empty());

    formatter.clear();

    assert_eq!(formatter.count(), 0);
    assert!(formatter.as_str().is_empty());
}

// TeeSpout Extended Tests

#[test]
fn test_tee_spout_inner_mut() {
    let collect = CollectSpout::<Frame>::new();
    let counter = CountingSpout::new();
    let mut tee = TeeSpout::new(collect, counter);

    let _ = tee.send(Frame::message("test"));

    let (a, b) = tee.inner_mut();
    assert_eq!(a.items().len(), 1);
    assert_eq!(b.count(), 1);
}

// LogSpout Tests

#[test]
fn test_log_spout() {
    let mut logged = alloc::vec::Vec::new();
    let mut spout = LogSpout(|frame: Frame| {
        logged.push(alloc::string::String::from(frame.msg()));
    });

    let _ = spout.send(Frame::message("first"));
    let _ = spout.send(Frame::message("second"));

    assert_eq!(logged.len(), 2);
    assert_eq!(logged[0], "first");
    assert_eq!(logged[1], "second");
}

// Actionable Blanket Impl Tests

#[test]
fn test_actionable_ref() {
    let err = TestError::temporary("ref test");
    let r: &dyn Actionable = &err;
    assert_eq!(r.status_value(), ErrorStatusValue::Temporary);
    assert!(r.is_retryable());
}

#[test]
fn test_actionable_box() {
    let err = TestError::permanent("box test");
    let b: alloc::boxed::Box<dyn Actionable> = alloc::boxed::Box::new(err);
    assert_eq!(b.status_value(), ErrorStatusValue::Permanent);
    assert!(!b.is_retryable());
}

// assert_depth / assert_origin Tests

#[test]
fn test_assert_depth_passes() {
    let ctx = Context::new(TestError::temporary("test"))
        .with_ctx("frame 1")
        .with_ctx("frame 2")
        .assert_depth(2);

    assert_eq!(ctx.frames().len(), 2);
}

#[test]
fn test_assert_depth_zero() {
    let ctx = Context::new(TestError::temporary("test")).assert_depth(0);
    assert!(ctx.frames().is_empty());
}

#[test]
fn test_assert_origin_passes() {
    let ctx = Context::new(TestError::temporary("test"))
        .with_ctx("from module")
        .assert_origin("alloc.rs");

    assert_eq!(ctx.frames().len(), 1);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "insufficient context")]
fn test_assert_depth_panics_when_insufficient() {
    let _ = Context::new(TestError::temporary("test"))
        .with_ctx("only one")
        .assert_depth(3);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "missing provenance")]
fn test_assert_origin_panics_when_missing() {
    let _ = Context::new(TestError::temporary("test"))
        .with_ctx("from somewhere")
        .assert_origin("nonexistent_module.rs");
}

// RetryOutcome Tests

#[test]
fn test_retry_outcome_inner() {
    let result: Result<(), _> =
        with_retry(1, || Err(Context::new(TestError::temporary("inner test"))));
    let outcome = result.unwrap_err();
    assert_eq!(outcome.inner().message, "inner test");
}

#[test]
fn test_retry_outcome_frames() {
    let result: Result<(), _> = with_retry(2, || {
        Err(Context::new(TestError::temporary("test")).with_ctx("some context"))
    });
    let outcome = result.unwrap_err();
    // Each attempt adds "some context" + "attempt N/2" annotation
    assert_eq!(outcome.frames().len(), 2);
    assert!(outcome.frames()[0].msg().contains("some context"));
    assert!(outcome.frames()[1].msg().contains("attempt 2/2"));
}

#[test]
fn test_retry_outcome_display() {
    let result: Result<(), _> = with_retry(1, || {
        Err(Context::new(TestError::temporary("display test")))
    });
    let outcome = result.unwrap_err();
    let display = format!("{outcome}");
    assert!(display.contains("display test"));
}

#[test]
fn test_retry_outcome_debug() {
    let result: Result<(), _> =
        with_retry(1, || Err(Context::new(TestError::permanent("debug test"))));
    let outcome = result.unwrap_err();
    let debug = format!("{outcome:?}");
    assert!(debug.contains("Permanent"));
}

#[test]
fn test_retry_max_attempts_zero_clamps_to_one() {
    let mut attempts = 0;
    let result: Result<(), _> = with_retry(0, || {
        attempts += 1;
        Err(Context::new(TestError::temporary("test")))
    });
    assert!(result.is_err());
    assert_eq!(attempts, 1);
}

#[test]
fn test_retry_temporary_then_permanent() {
    let mut attempts = 0;
    let result: Result<(), _> = with_retry(5, || {
        attempts += 1;
        if attempts <= 2 {
            Err(Context::new(TestError::temporary("retry me")))
        } else {
            Err(Context::new(TestError::permanent("fatal")))
        }
    });

    assert_eq!(attempts, 3);
    let outcome = result.unwrap_err();
    assert!(outcome.is_permanent());
    assert_eq!(outcome.inner().message, "fatal");
}

#[test]
fn test_retry_outcome_error_source() {
    let result: Result<(), _> =
        with_retry(1, || Err(Context::new(TestError::permanent("source test"))));
    let outcome = result.unwrap_err();
    let source = core::error::Error::source(&outcome);
    assert!(source.is_some());
    assert_eq!(format!("{}", source.unwrap()), "source test");
}

// with_retry_delay Tests

#[cfg(feature = "std")]
#[test]
fn test_with_retry_delay_success() {
    let mut attempts = 0;
    let result = with_retry_delay(
        3,
        |_| std::time::Duration::from_millis(1),
        || {
            attempts += 1;
            if attempts < 2 {
                Err(Context::new(TestError::temporary("retry")))
            } else {
                Ok(42)
            }
        },
    );
    assert_eq!(result.unwrap(), 42);
    assert_eq!(attempts, 2);
}

#[cfg(feature = "std")]
#[test]
fn test_with_retry_delay_exhausted() {
    let mut attempts = 0;
    let result: Result<(), _> = with_retry_delay(
        2,
        |_| std::time::Duration::from_millis(1),
        || {
            attempts += 1;
            Err(Context::new(TestError::temporary("always fails")))
        },
    );
    assert!(result.unwrap_err().is_exhausted());
    assert_eq!(attempts, 2);
}

#[cfg(feature = "std")]
#[test]
fn test_with_retry_delay_permanent_stops() {
    let mut attempts = 0;
    let result: Result<(), _> = with_retry_delay(
        5,
        |_| std::time::Duration::from_millis(1),
        || {
            attempts += 1;
            Err(Context::new(TestError::permanent("fatal")))
        },
    );
    assert!(result.unwrap_err().is_permanent());
    assert_eq!(attempts, 1);
}

// exponential_backoff Tests

#[cfg(feature = "std")]
#[test]
fn test_exponential_backoff() {
    let base = std::time::Duration::from_millis(100);
    let max = std::time::Duration::from_secs(5);
    let mut backoff = exponential_backoff(base, max);

    assert_eq!(backoff(0), base); // attempt 0 treated as 1
    assert_eq!(backoff(1), base); // 100ms * 2^0 = 100ms
    assert_eq!(backoff(2), std::time::Duration::from_millis(200)); // 100ms * 2^1
    assert_eq!(backoff(3), std::time::Duration::from_millis(400)); // 100ms * 2^2
    assert_eq!(backoff(4), std::time::Duration::from_millis(800)); // 100ms * 2^3
}

#[cfg(feature = "std")]
#[test]
fn test_exponential_backoff_caps_at_max() {
    let base = std::time::Duration::from_millis(100);
    let max = std::time::Duration::from_millis(500);
    let mut backoff = exponential_backoff(base, max);

    assert_eq!(backoff(10), max);
    assert_eq!(backoff(31), max);
}

// actionable! Macro Tests

#[test]
fn test_actionable_macro_temporary() {
    #[derive(Debug)]
    struct TempErr;
    actionable!(TempErr, Temporary);

    assert!(TempErr.is_retryable());
    assert_eq!(TempErr.status_value(), ErrorStatusValue::Temporary);
}

#[test]
fn test_actionable_macro_permanent() {
    #[derive(Debug)]
    struct PermErr;
    actionable!(PermErr, Permanent);

    assert!(!PermErr.is_retryable());
    assert_eq!(PermErr.status_value(), ErrorStatusValue::Permanent);
}

#[test]
fn test_actionable_macro_custom() {
    #[derive(Debug)]
    struct CustomErr(bool);
    actionable!(CustomErr, self => {
        if self.0 {
            ErrorStatusValue::Temporary
        } else {
            ErrorStatusValue::Permanent
        }
    });

    assert!(CustomErr(true).is_retryable());
    assert!(!CustomErr(false).is_retryable());
}

// display_error! Macro Tests

#[test]
fn test_display_error_unit_variant() {
    display_error! {
        pub enum UnitErr {
            #[display("not found")]
            NotFound,
            #[display("timed out")]
            Timeout,
        }
    }

    assert_eq!(format!("{}", UnitErr::NotFound), "not found");
    assert_eq!(format!("{}", UnitErr::Timeout), "timed out");
    // Verify Error trait is implemented
    let _: &dyn core::error::Error = &UnitErr::NotFound;
}

#[test]
fn test_display_error_struct_variant() {
    display_error! {
        pub enum StructErr {
            #[display("checksum mismatch: expected {expected:#x}, got {actual:#x}")]
            Checksum { expected: u32, actual: u32 },
            #[display("I/O error")]
            Io,
        }
    }

    let err = StructErr::Checksum {
        expected: 0xAB,
        actual: 0xCD,
    };
    assert_eq!(
        format!("{err}"),
        "checksum mismatch: expected 0xab, got 0xcd"
    );
    assert_eq!(format!("{}", StructErr::Io), "I/O error");
    // Debug is derived
    let debug = alloc::format!("{err:?}");
    assert!(debug.contains("Checksum"));
}

// Generic display_error! Tests

#[test]
fn test_display_error_generic_single_param() {
    display_error! {
        pub enum SingleGeneric<E: core::fmt::Debug + core::fmt::Display> {
            #[display("inner: {source}")]
            Inner { source: E },
            #[display("plain error")]
            Plain,
        }
    }

    let err = SingleGeneric::<alloc::string::String>::Inner {
        source: alloc::string::String::from("boom"),
    };
    assert_eq!(format!("{err}"), "inner: boom");
    assert_eq!(
        format!("{}", SingleGeneric::<alloc::string::String>::Plain),
        "plain error"
    );
    let _: &dyn core::error::Error = &err;
}

#[test]
fn test_display_error_generic_two_params() {
    display_error! {
        pub enum TwoGeneric<Id: core::fmt::Debug, E: core::fmt::Debug + core::fmt::Display> {
            #[display("not found: {id:?}")]
            NotFound { id: Id },
            #[display("failed: {source}")]
            Failed { source: E },
            #[display("both: {id:?} => {source}")]
            Both { id: Id, source: E },
        }
    }

    let err = TwoGeneric::<u32, alloc::string::String>::NotFound { id: 42 };
    assert_eq!(format!("{err}"), "not found: 42");

    let err = TwoGeneric::<u32, alloc::string::String>::Failed {
        source: alloc::string::String::from("oops"),
    };
    assert_eq!(format!("{err}"), "failed: oops");

    let err = TwoGeneric::<u32, alloc::string::String>::Both {
        id: 7,
        source: alloc::string::String::from("bad"),
    };
    assert_eq!(format!("{err}"), "both: 7 => bad");
}

#[test]
fn test_display_error_generic_multi_bound() {
    display_error! {
        pub enum MultiBound<E: core::fmt::Debug + core::fmt::Display> {
            #[display("err: {inner}")]
            Wrapped { inner: E },
        }
    }

    let err = MultiBound::<alloc::string::String>::Wrapped {
        inner: alloc::string::String::from("test"),
    };
    assert_eq!(format!("{err}"), "err: test");
    // Debug also works because of #[derive(Debug)] + E: Debug
    let _ = alloc::format!("{err:?}");
}

// Result Extension on Ok Path Tests

#[test]
fn test_wrap_ctx_ok_passes_through() {
    let result: Result<i32, TestError> = Ok(42);
    assert_eq!(result.wrap_ctx("context").unwrap(), 42);
}

#[test]
fn test_with_ctx_ok_passes_through() {
    let result: Result<i32, Context<TestError>> = Ok(42);
    assert_eq!(result.with_ctx("context").unwrap(), 42);
}

// bail! / ensure! Tests

#[test]
fn test_bail() {
    fn inner() -> Result<(), Context<TestError>> {
        bail!(TestError::permanent("bail test"));
    }
    let err = inner().unwrap_err();
    assert_eq!(err.inner().message, "bail test");
}

#[test]
fn test_bail_ok_path() {
    fn inner(fail: bool) -> Result<i32, Context<TestError>> {
        if fail {
            bail!(TestError::permanent("fail"));
        }
        Ok(42)
    }
    assert_eq!(inner(false).unwrap(), 42);
    assert!(inner(true).is_err());
}

#[test]
fn test_ensure_passes() {
    fn inner(x: i32) -> Result<i32, Context<TestError>> {
        ensure!(x > 0, TestError::permanent("must be positive"));
        Ok(x)
    }
    assert_eq!(inner(5).unwrap(), 5);
}

#[test]
fn test_ensure_fails() {
    fn inner(x: i32) -> Result<i32, Context<TestError>> {
        ensure!(x > 0, TestError::permanent("must be positive"));
        Ok(x)
    }
    let err = inner(-1).unwrap_err();
    assert_eq!(err.inner().message, "must be positive");
}

// OptionExt Tests

#[test]
fn test_option_wrap_ctx_none() {
    let opt: Option<i32> = None;
    let err = opt
        .wrap_ctx(TestError::temporary("missing"), "value was none")
        .unwrap_err();
    assert_eq!(err.inner().message, "missing");
    assert_eq!(err.frames().len(), 1);
    assert!(err.frames()[0].msg().contains("value was none"));
}

#[test]
fn test_option_wrap_ctx_some() {
    let opt: Option<i32> = Some(42);
    let val = opt
        .wrap_ctx(TestError::temporary("missing"), "value was none")
        .unwrap();
    assert_eq!(val, 42);
}

#[test]
fn test_option_wrap_ctx_lazy_none() {
    let opt: Option<i32> = None;
    let err = opt
        .wrap_ctx_lazy(|| {
            (
                TestError::temporary("lazy missing"),
                format!("lazy {}", "msg"),
            )
        })
        .unwrap_err();
    assert_eq!(err.inner().message, "lazy missing");
    assert!(err.frames()[0].msg().contains("lazy msg"));
}

#[test]
fn test_option_wrap_ctx_lazy_some() {
    let mut called = false;
    let opt: Option<i32> = Some(42);
    let val = opt
        .wrap_ctx_lazy(|| {
            called = true;
            (TestError::temporary("unused"), "unused".into())
        })
        .unwrap();
    assert_eq!(val, 42);
    assert!(!called);
}

// Backtrace Tests

#[cfg(feature = "std")]
#[test]
fn test_backtrace_accessor() {
    let ctx = Context::new(TestError::temporary("test"));
    let bt = ctx.backtrace();
    // Without RUST_BACKTRACE set, status is Disabled
    assert!(
        bt.status() == std::backtrace::BacktraceStatus::Disabled
            || bt.status() == std::backtrace::BacktraceStatus::Captured
    );
}

#[cfg(feature = "std")]
#[test]
fn test_backtrace_survives_resolve() {
    let ctx = Context::new(TestError::temporary("test"));
    let Resolved::Temporary(temp) = ctx.resolve() else {
        panic!("expected Temporary");
    };
    let bt = temp.backtrace();
    assert!(
        bt.status() == std::backtrace::BacktraceStatus::Disabled
            || bt.status() == std::backtrace::BacktraceStatus::Captured
    );
}

#[cfg(feature = "std")]
#[test]
fn test_backtrace_survives_exhaust() {
    let ctx = Context::new(TestError::temporary("test"));
    let Resolved::Temporary(temp) = ctx.resolve() else {
        panic!("expected Temporary");
    };
    let exhausted = temp.exhaust();
    let bt = exhausted.backtrace();
    assert!(
        bt.status() == std::backtrace::BacktraceStatus::Disabled
            || bt.status() == std::backtrace::BacktraceStatus::Captured
    );
}

#[cfg(feature = "std")]
#[test]
fn test_backtrace_on_bounded() {
    let ctx = Context::bounded(TestError::temporary("test"), 5);
    let bt = ctx.backtrace();
    assert!(
        bt.status() == std::backtrace::BacktraceStatus::Disabled
            || bt.status() == std::backtrace::BacktraceStatus::Captured
    );
}

#[cfg(feature = "std")]
#[test]
fn test_backtrace_on_bounded_collect() {
    let ctx = Context::bounded_collect(TestError::temporary("test"), 5);
    let bt = ctx.backtrace();
    assert!(
        bt.status() == std::backtrace::BacktraceStatus::Disabled
            || bt.status() == std::backtrace::BacktraceStatus::Captured
    );
}

#[cfg(feature = "std")]
#[test]
fn test_backtrace_in_display() {
    let ctx = Context::new(TestError::temporary("root cause"));
    // We can't control RUST_BACKTRACE in tests, but we can verify the Display
    // path doesn't panic regardless of backtrace status
    let display = format!("{ctx}");
    assert!(display.contains("root cause"));
}

// Send/Sync Static Assertions

const _: () = {
    // Compile-time proof that T: Send + Sync. The transmute is never called;
    // the compiler just needs to verify the trait bounds at monomorphization.
    const fn is_send_sync<T: Send + Sync>() {}

    is_send_sync::<Frame>();
    is_send_sync::<Context<TestError>>();
    is_send_sync::<RetryOutcome<TestError>>();
    is_send_sync::<FrameFormatter>();
    is_send_sync::<CountingSpout>();
};

// Bytecast serialization tests

#[cfg(feature = "bytecast")]
mod bytecast_tests {
    use super::*;
    use bytecast::{FromBytes, ToBytesExt};

    // TestError with bytecast support for round-trip testing
    #[derive(Debug, Clone, bytecast::DeriveToBytes, bytecast::DeriveFromBytes)]
    struct SerTestError {
        status_val: u8,
        message: alloc::string::String,
    }

    impl core::fmt::Display for SerTestError {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(f, "{}", self.message)
        }
    }

    impl core::error::Error for SerTestError {}

    impl Actionable for SerTestError {
        fn status_value(&self) -> ErrorStatusValue {
            ErrorStatusValue::from_u32(self.status_val as u32).unwrap_or_default()
        }
    }

    #[test]
    fn test_status_value_round_trip() {
        let values = [
            ErrorStatusValue::Permanent,
            ErrorStatusValue::Temporary,
            ErrorStatusValue::Exhausted,
        ];
        for val in &values {
            let bytes = val.to_vec().unwrap();
            let (decoded, consumed) = ErrorStatusValue::from_bytes(&bytes).unwrap();
            assert_eq!(*val, decoded);
            assert_eq!(consumed, bytes.len());
        }
    }

    #[test]
    fn test_frame_round_trip() {
        let frame = Frame::new("src/main.rs", 42, 10, "something failed");
        let bytes = frame.to_vec().unwrap();
        let (decoded, consumed) = Frame::from_bytes(&bytes).unwrap();

        assert_eq!(decoded.file(), "src/main.rs");
        assert_eq!(decoded.line(), 42);
        assert_eq!(decoded.column(), 10);
        assert_eq!(decoded.msg(), "something failed");
        assert_eq!(consumed, bytes.len());
    }

    #[test]
    fn test_frame_unknown_location_round_trip() {
        let frame = Frame::message("just a message");
        let bytes = frame.to_vec().unwrap();
        let (decoded, _) = Frame::from_bytes(&bytes).unwrap();

        assert_eq!(decoded.file(), "<unknown>");
        assert_eq!(decoded.line(), 0);
        assert_eq!(decoded.column(), 0);
        assert_eq!(decoded.msg(), "just a message");
    }

    #[test]
    fn test_contextualized_round_trip() {
        let err = SerTestError {
            status_val: 1,
            message: alloc::string::String::from("timeout"),
        };
        let ctx = Context::new(err)
            .with_ctx("connecting to db")
            .with_ctx("handling request");

        let bytes = ctx.to_vec().unwrap();
        let (decoded, consumed): (Context<SerTestError>, usize) =
            FromBytes::from_bytes(&bytes).unwrap();

        assert_eq!(decoded.inner().message, "timeout");
        assert_eq!(decoded.inner().status_val, 1);
        assert_eq!(decoded.frames().len(), 2);
        assert!(decoded.frames()[0].msg().contains("connecting to db"));
        assert!(decoded.frames()[1].msg().contains("handling request"));
        assert_eq!(decoded.overflow_count(), 0);
        assert_eq!(consumed, bytes.len());
    }

    #[test]
    fn test_contextualized_skipped_fields_get_defaults() {
        let err = SerTestError {
            status_val: 0,
            message: alloc::string::String::from("not found"),
        };
        let ctx = Context::new(err).with_ctx("lookup");

        let bytes = ctx.to_vec().unwrap();
        let (decoded, _): (Context<SerTestError>, _) = FromBytes::from_bytes(&bytes).unwrap();

        // Overflow should be DropSpout (default)
        assert!(!decoded.has_overflow());
        // Status should be Dynamic (deserialized type)
        assert_eq!(decoded.status_value(), ErrorStatusValue::Permanent);
    }

    #[test]
    fn test_contextualized_with_overflow_round_trip() {
        let err = SerTestError {
            status_val: 1,
            message: alloc::string::String::from("retry me"),
        };
        let ctx = Context::bounded(err, 2)
            .with_ctx("frame 1")
            .with_ctx("frame 2")
            .with_ctx("frame 3");

        assert_eq!(ctx.overflow_count(), 1);
        assert_eq!(ctx.frames().len(), 2);

        let bytes = ctx.to_vec().unwrap();
        let (decoded, _): (Context<SerTestError>, _) = FromBytes::from_bytes(&bytes).unwrap();

        // Frames and overflow_count are preserved
        assert_eq!(decoded.frames().len(), 2);
        assert_eq!(decoded.overflow_count(), 1);
        assert!(decoded.frames()[0].msg().contains("frame 2"));
        assert!(decoded.frames()[1].msg().contains("frame 3"));
    }

    #[test]
    fn test_decode_context_temporary() {
        let err = SerTestError {
            status_val: 1,
            message: alloc::string::String::from("timeout"),
        };
        let ctx = Context::new(err);
        let Resolved::Temporary(temp) = ctx.resolve() else {
            panic!("expected Temporary");
        };

        let bytes = temp.to_vec().unwrap();
        let (decoded, consumed) = decode_context::<SerTestError>(&bytes).unwrap();

        assert_eq!(consumed, bytes.len());
        match decoded {
            DecodedContext::Temporary(c) => {
                assert!(c.is_retryable());
                assert_eq!(c.inner().message, "timeout");
            }
            _ => panic!("expected Temporary variant"),
        }
    }

    #[test]
    fn test_decode_context_permanent() {
        let err = SerTestError {
            status_val: 0,
            message: alloc::string::String::from("not found"),
        };
        let ctx = Context::new(err);
        let Resolved::Permanent(perm) = ctx.resolve() else {
            panic!("expected Permanent");
        };

        let bytes = perm.to_vec().unwrap();
        let (decoded, consumed) = decode_context::<SerTestError>(&bytes).unwrap();

        assert_eq!(consumed, bytes.len());
        match decoded {
            DecodedContext::Permanent(c) => {
                assert!(!c.is_retryable());
                assert_eq!(c.inner().message, "not found");
            }
            _ => panic!("expected Permanent variant"),
        }
    }

    #[test]
    fn test_decode_context_exhausted() {
        let err = SerTestError {
            status_val: 1,
            message: alloc::string::String::from("gave up"),
        };
        let ctx = Context::new(err);
        let Resolved::Temporary(temp) = ctx.resolve() else {
            panic!("expected Temporary");
        };
        let exhausted = temp.exhaust();

        let bytes = exhausted.to_vec().unwrap();
        let (decoded, consumed) = decode_context::<SerTestError>(&bytes).unwrap();

        assert_eq!(consumed, bytes.len());
        match decoded {
            DecodedContext::Exhausted(c) => {
                assert!(!c.is_retryable());
                assert_eq!(c.inner().message, "gave up");
            }
            _ => panic!("expected Exhausted variant"),
        }
    }

    #[test]
    fn test_decode_context_preserves_frames() {
        let err = SerTestError {
            status_val: 1,
            message: alloc::string::String::from("timeout"),
        };
        let ctx = Context::new(err)
            .with_ctx("connecting to db")
            .with_ctx("handling request");
        let Resolved::Temporary(temp) = ctx.resolve() else {
            panic!("expected Temporary");
        };

        let bytes = temp.to_vec().unwrap();
        let (decoded, _) = decode_context::<SerTestError>(&bytes).unwrap();

        match decoded {
            DecodedContext::Temporary(c) => {
                assert_eq!(c.frames().len(), 2);
                assert!(c.frames()[0].msg().contains("connecting to db"));
                assert!(c.frames()[1].msg().contains("handling request"));
            }
            _ => panic!("expected Temporary variant"),
        }
    }
}

// LogRecord tests (serde)

#[cfg(feature = "serde")]
mod serde_tests {
    use super::*;

    #[test]
    fn test_log_record_from_contextualized() {
        let err = Context::new(TestError::temporary("timeout"))
            .with_ctx("connecting to db")
            .with_ctx("handling request");

        let record = LogRecord::from(&err);

        assert_eq!(record.error, "timeout");
        assert_eq!(record.status, "temporary");
        assert!(record.retryable);
        assert_eq!(record.overflow_count, 0);
        assert_eq!(record.frames.len(), 2);
        assert_eq!(record.frames[0].message, "connecting to db");
        assert_eq!(record.frames[1].message, "handling request");
    }

    #[test]
    fn test_log_record_serde_json() {
        let err = Context::new(TestError::temporary("timeout")).with_ctx("connecting to db");

        let record = LogRecord::from(&err);
        let json: serde_json::Value = serde_json::to_value(&record).unwrap();

        assert_eq!(json["error"], "timeout");
        assert_eq!(json["status"], "temporary");
        assert_eq!(json["retryable"], true);
        assert_eq!(json["frames"][0]["message"], "connecting to db");
    }

    #[test]
    fn test_log_record_permanent() {
        let err = Context::new(TestError::permanent("not found")).with_ctx("lookup");
        let record = LogRecord::from(&err);

        assert_eq!(record.status, "permanent");
        assert!(!record.retryable);
    }

    #[test]
    fn test_log_record_with_overflow() {
        let err = Context::bounded(TestError::temporary("test"), 2)
            .with_ctx("frame 1")
            .with_ctx("frame 2")
            .with_ctx("frame 3");

        let record = LogRecord::from(&err);

        assert_eq!(record.frames.len(), 2);
        assert_eq!(record.overflow_count, 1);
    }

    #[test]
    fn test_log_record_serde_round_trip() {
        let err = Context::new(TestError::temporary("timeout")).with_ctx("connecting to db");

        let record = LogRecord::from(&err);
        let json = serde_json::to_string(&record).unwrap();
        let decoded: LogRecord = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.error, record.error);
        assert_eq!(decoded.status, record.status);
        assert_eq!(decoded.retryable, record.retryable);
        assert_eq!(decoded.frames.len(), record.frames.len());
    }
}

// LogRecord tests (facet)

#[cfg(feature = "facet")]
mod facet_tests {
    use super::*;

    #[test]
    fn test_log_record_facet_json() {
        let err = Context::new(TestError::temporary("timeout")).with_ctx("connecting to db");

        let record = LogRecord::from(&err);
        let json = facet_json::to_string(&record).unwrap();

        assert!(json.contains("\"timeout\""));
        assert!(json.contains("\"temporary\""));
        assert!(json.contains("\"connecting to db\""));
    }

    #[test]
    fn test_log_record_facet_round_trip() {
        let err = Context::new(TestError::temporary("timeout")).with_ctx("connecting to db");

        let record = LogRecord::from(&err);
        let json = facet_json::to_string(&record).unwrap();
        let decoded: LogRecord = facet_json::from_str(&json).unwrap();

        assert_eq!(decoded.error, record.error);
        assert_eq!(decoded.status, record.status);
        assert_eq!(decoded.retryable, record.retryable);
        assert_eq!(decoded.frames.len(), record.frames.len());
    }
}
