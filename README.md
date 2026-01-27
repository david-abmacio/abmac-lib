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
- **MPSC aggregation** - Multiple producers write independently, merge on demand

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
- Default: single-threaded, maximum performance (~5 billion elem/sec)
- `atomics` feature: lock-free SPSC (single-producer, single-consumer)
- `MpscRing`: zero-contention multi-producer support

**No-Std Support**
- Works in embedded environments (requires `alloc` for some sinks)
- Default uses `Cell` for maximum performance
- `atomics` feature enables thread-safe SPSC access

**Zero-Copy Custom Serialization**
- Built-in `ToBytes`/`FromBytes` traits for primitive types
- `ByteSerializer` for serializing items to byte buffers

## Installation

***Note: This library is not crates.io yet. Please use the repository version until it is published.***

```toml
[dependencies]
spill-ring = "0.1"
```

For SPSC (single-producer, single-consumer) thread-safe access:

```toml
[dependencies]
spill-ring = { version = "0.1", features = ["atomics"] }
```

## Thread Safety

By default, SpillRing is single-threaded for maximum performance. Enable the `atomics` feature for SPSC (single-producer, single-consumer) concurrent use:

```rust
// Requires: spill-ring = { version = "0.1", features = ["atomics"] }
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

For multiple producers, use `MpscRing` for zero-contention multi-producer support (no `atomics` feature needed).

## Multiple Producers (MPSC)

When order between producers doesn't matter, `MpscRing` provides zero-overhead multi-producer support. Each producer owns an independent ring running at full no-atomics speed with no contention:

```rust
use std::thread;
use spill_ring::{MpscRing, CollectSink, collect};

// Create 4 producers and a consumer
let (producers, mut consumer) = MpscRing::<u64, 1024>::with_consumer(4);

// Each producer runs on its own thread at full speed
let finished: Vec<_> = thread::scope(|s| {
    producers
        .into_iter()
        .map(|producer| {
            s.spawn(move || {
                for i in 0..100_000 {
                    producer.push(i);
                }
                producer // Return producer for collection
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect()
});

// Merge producers back and drain all items
collect(finished, &mut consumer);
let mut sink = CollectSink::new();
consumer.drain(&mut sink);
// Process items (order across producers not guaranteed)
```

 This pattern is ideal for large items where parallel cache loading outweighs coordination overhead.

## Performance

**Benchmarks** (AMD Ryzen 7 7840U, Linux 6.18, Rust 1.85):

| Mode | Push Throughput | Use Case |
|------|-----------------|----------|
| Default | 5.5 billion elem/sec | Single-threaded (maximum performance) |
| `atomics` feature | 229 million elem/sec | SPSC concurrent access |

The default mode uses `Cell` for single-threaded access with no atomic overhead. Enable `atomics` when you need thread-safe SPSC access.

**Comparison to VecDeque:**

| Implementation | Push Throughput |
|----------------|-----------------|
| SpillRing (default) | 5.5 billion elem/sec |
| VecDeque (manual eviction) | 765 million elem/sec |
| SpillRing (atomics) | 229 million elem/sec |

SpillRing's power-of-two capacity enables fast bitwise modulo, making the default version ~7x faster than `VecDeque` with equivalent eviction logic.

**Cache effects** (50k push operations, default mode):

| Capacity | Throughput | Notes |
|----------|------------|-------|
| 16 | 5.5 billion elem/sec | Fits L1 cache |
| 4096 | 5.2 billion elem/sec | L2 cache |
| 65536 | 4.8 billion elem/sec | L3 cache |

Run benchmarks locally: `cargo bench`

## Ring Chaining

Since `SpillRing` implements `Sink`, rings can chain together. Overflow from one ring flows into the next, creating tiered buffers:

```rust
use spill_ring::SpillRing;

// Tier 2: larger buffer, overflows are dropped
let l2: SpillRing<i32, 64> = SpillRing::new();

// Tier 1: small hot buffer, overflows go to L2
let l1: SpillRing<i32, 8, _> = SpillRing::with_sink(l2);

// Push through L1
for i in 0..100 {
    l1.push(i);
}

// L1 holds [92..100] (most recent 8)
// L1's sink (L2) holds [36..92] (next 56)
// Earlier items dropped when L2 overflowed
```

This pattern extends to any depth. Replace the final `DropSink` with storage, logging, or any other sink to preserve all data:

```rust
// L3: disk storage (never drops)
let storage = DiskSink::new("events.log");

// L2: warm buffer
let l2: SpillRing<Event, 1024, _> = SpillRing::with_sink(storage);

// L1: hot buffer  
let l1: SpillRing<Event, 64, _> = SpillRing::with_sink(l2);
```

Recent items stay in fast L1 cache, older items in L2, oldest items persist to disk.

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
