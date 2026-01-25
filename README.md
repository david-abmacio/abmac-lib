# Spill-Ring

A fixed-capacity ring buffer that spills to a configurable sink on overflow.

## Why Spill-Ring

Bounded buffers usually force: block when full, drop new items, or drop old items. Spill-Ring evicts the oldest item to a *sink* rather than drop. This preserves all data while maintaining bounded memory. If the sink is not configured, the items are dropped as expected.

This pattern is useful when recent items need fast access but older items should flow somewhere else like to disk, over a network, or into another processing stage.

## When To Use It

- **In-memory checkpoint systems** - Keep recent checkpoints in memory, spill older ones to storage
- **Event buffers** - Hot events stay accessible, cold events archive automatically
- **Logging pipelines** - Buffer recent logs, flush older entries to disk
- **Embedded systems** - Fixed memory footprint with `no_std` support
- **SPSC queues** - Lock-free single-producer, single-consumer with overflow handling

## What It Does

```rust
use spill_ring::SpillRing;

// Ring buffer with capacity 4, overflow is dropped (default sink)
let ring: SpillRing<i32, 4> = SpillRing::new();

ring.push(1);
ring.push(2);
ring.push(3);
ring.push(4);  // Buffer is now full

ring.push(5);  // Evicts 1 (oldest), buffer contains [2, 3, 4, 5]

assert_eq!(ring.pop(), Some(2));
```

With a custom sink to capture evicted items:

```rust
use spill_ring::{SpillRing, CollectSink};

let sink = CollectSink::new();
let ring: SpillRing<i32, 4, _> = SpillRing::with_sink(sink);

for i in 1..=10 {
    ring.push(i);
}

// Ring contains [7, 8, 9, 10]
// Sink captured [1, 2, 3, 4, 5, 6]
```

## Features

**Configurable Sinks**
- `DropSink` - Discard evicted items (default)
- `CollectSink` - Gather evicted items into a `Vec`
- `FnSink` - Call a closure on each eviction
- Custom sinks via the `Sink` trait

**Thread Safety**
- Lock-free SPSC (single-producer, single-consumer) when using atomics
- One thread pushes, another pops, no mutex needed
- Interior mutability via atomics and seqlock coordination

**No-Std Support**
- Works in embedded environments (requires `alloc` for some sinks)
- `no-atomics` feature for single-context systems using `Cell`

**Zero-Copy Custom Serialization**
- Built-in `ToBytes`/`FromBytes` traits for primitive types
- `ByteSerializer` for serializing items to byte buffers

## Installation

***Note: This library is not crates.io yet. Please use the repository version until it is published.***

```toml
[dependencies]
spill-ring = "0.1"
```

For embedded systems without atomics:

```toml
[dependencies]
spill-ring = { version = "0.1", features = ["no-atomics"] }
```

## Thread Safety

SpillRing is safe for single-producer, single-consumer (SPSC) concurrent use:

```rust
use std::sync::Arc;
use std::thread;
use spill_ring::SpillRing;

let ring = Arc::new(SpillRing::<u64, 256>::new());

let producer = Arc::clone(&ring);
let consumer = Arc::clone(&ring);

thread::spawn(move || {
    for i in 0..1000 {
        producer.push(i);
    }
});

thread::spawn(move || {
    while let Some(item) = consumer.pop() {
        // Process item
    }
});
```

For multiple producers or multiple consumers, wrap in a `Mutex`.

## Performance

SpillRing has two modes with different performance characteristics.

**Benchmarks** (AMD Ryzen 7 7840U, Linux 6.18, Rust 1.85):

| Mode | Push Throughput | Use Case |
|------|-----------------|----------|
| Default (atomics) | 193 Melem/s | SPSC concurrent access |
| `no-atomics` feature | 4.6 Gelem/s | Single-threaded |

The default mode uses atomic operations and a seqlock for thread-safe SPSC access. This adds overhead compared to single-threaded use.

For single-threaded applications, enable `no-atomics` for ~20x better throughput:

```toml
[dependencies]
spill-ring = { version = "0.1", features = ["no-atomics"] }
```

**Comparison to VecDeque:**

| Implementation | Push Throughput |
|----------------|-----------------|
| SpillRing (no-atomics) | 4.6 Gelem/s |
| VecDeque (manual eviction) | 632 Melem/s |
| SpillRing (atomics) | 193 Melem/s |

SpillRing's power-of-two capacity enables fast bitwise modulo, making the `no-atomics` version faster than `VecDeque` with equivalent eviction logic.

**Cache effects** (50k push operations):

| Capacity | Throughput | Notes |
|----------|------------|-------|
| 16 | 192 Melem/s | Fits L1 cache |
| 4096 | 177 Melem/s | L2 cache |
| 65536 | 141 Melem/s | L3 cache |

Run benchmarks locally: `cargo bench`

## Custom Sinks

Implement `Sink` to control where evicted items go:

```rust
use spill_ring::{SpillRing, Sink};

struct LogSink;

impl<T: std::fmt::Debug> Sink<T> for LogSink {
    fn send(&mut self, item: T) {
        eprintln!("evicted: {:?}", item);
    }
    
    fn flush(&mut self) {}
}

let ring: SpillRing<String, 8, LogSink> = SpillRing::with_sink(LogSink);
```

## Project Structure

- `spill-ring` - Main crate re-exporting everything
- `spill-ring-core` - Core ring buffer implementation (`no_std`)
- `spill-ring-macros` - Procedural macros enabled with the **macros** feature.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
