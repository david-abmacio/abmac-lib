# verdict

Typestate-driven error handling.

Most error libraries focus on *what went wrong*. Verdict focuses on *what to do about it* — encoding retryability directly into the type system so the compiler enforces correct error handling at every call site.

## Getting Started

```rust
use verdict::prelude::*;

#[derive(Debug)]
struct ApiError { status: u16 }

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "API error {}", self.status)
    }
}

impl std::error::Error for ApiError {}

// One line to declare retryability
actionable!(ApiError, self => {
    if self.status == 429 || self.status >= 500 {
        ErrorStatusValue::Temporary
    } else {
        ErrorStatusValue::Permanent
    }
});

// Even simpler for fixed-status errors
actionable!(std::io::Error, Temporary);
```

## Context Tracking

Wrap errors with location-tracked context frames. Every `wrap_ctx` call captures file, line, and column automatically.

```rust
fn fetch_user(id: u32) -> Result<String, Ctx<ApiError>> {
    api_call(id).wrap_ctx("fetching user profile")
}

fn load_dashboard() -> Result<String, Ctx<ApiError>> {
    fetch_user(42).with_ctx("loading dashboard")
}

// Output:
// API error 503
//   |-> fetching user profile, at src/api.rs:12:5
//   |-> loading dashboard, at src/main.rs:8:5
```

Propagating errors with the `?` operator works — `From<E>` wraps the error automatically. The tradeoff is no context frame, but the backtrace is captured.

```rust
fn get_user(id: u32) -> Result<String, Ctx<ApiError>> {
    Ok(api_call(id)?)
}
```

## Backtrace

With the `std` feature, every `Context` captures a backtrace at construction time. Backtraces are zero-cost when `RUST_BACKTRACE` is not set and appear automatically in `Display` output when enabled:

```
API error 503
  |-> fetching user profile, at src/api.rs:12:5
  |-> loading dashboard, at src/main.rs:8:5

   0: my_app::api::fetch_user
   1: my_app::main
   ...
```

Context frames give you the *semantic* path. Backtraces give you the *code* path.

## Typestate Safety

Errors carry compile-time status that prevents misuse:

```
       ┌─────────┐     ┌───────────┐     ┌───────────┐
       │ Dynamic │────►│ Temporary │────►│ Exhausted │
       └────┬────┘     └───────────┘     └───────────┘
            │           (retryable)     (retries exhausted)
            │
            └─────────►┌───────────┐
                       │ Permanent │
                       └───────────┘
                      (never retryable)
```

`resolve()` bridges runtime status to compile-time typestate:

```rust
let ctx: Ctx<ApiError> = /* ... */;

match ctx.resolve() {
    Resolved::Temporary(temp) => { /* Context<_, Temporary> — retryable */ }
    Resolved::Exhausted(ex) => { /* Context<_, Exhausted> — retries spent */ }
    Resolved::Permanent(perm) => { /* Context<_, Permanent> — never retryable */ }
}
```

## Retry

Built-in retry with automatic typestate transitions:

```rust
let result = with_retry(3, || {
    api_call().wrap_ctx("calling payment service")
});

match result {
    Ok(value) => println!("{value}"),
    Err(RetryOutcome::Permanent(e)) => eprintln!("not retryable: {e}"),
    Err(RetryOutcome::Exhausted(e)) => eprintln!("gave up after 3 attempts: {e}"),
}
```

With backoff:

```rust
let result = with_retry_delay(5, exponential_backoff(Duration::from_millis(100), Duration::from_secs(10)), || {
    api_call().wrap_ctx("calling payment service")
});
```

## Bounded Context

By default, context frames grow unbounded. For memory-sensitive applications:

```rust
// Keep last 8 frames, drop older ones
let err = Context::bounded(error, 8);

// Keep last 8 frames, collect older ones for inspection
let err = Context::bounded_collect(error, 8);
let evicted = err.into_overflow().into_items();
```

For custom overflow behavior (logging, channels, metrics), use `Context::with_overflow` with any [`Spout`](https://docs.rs/spout) implementation.

## Features

```toml
[dependencies]
verdict = "0.1"                                           # std (default)
verdict = { version = "0.1", default-features = false, features = ["alloc"] }  # no_std + heap
verdict = { version = "0.1", default-features = false }   # no_std core only
```

## License

Licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE) at your option.
