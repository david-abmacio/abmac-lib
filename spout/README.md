# spout

A minimal, composable push-based trait for sending items to a destination. The inverse of iterators — push instead of pull.

## Features

- **no_std by default** — works on embedded devices with just `alloc`
- **Zero dependencies** — pure Rust, no external crates
- **Composable** — spouts wrap spouts for batching, reduction, and transformation
- **Simple API** — just `send()`, `send_all()`, and `flush()`

## Spout Types

| Type | Description | Requires |
|------|-------------|----------|
| `DropSpout` | Drops all items (default no-op spout) | — |
| `CollectSpout<T>` | Collects items into a `Vec<T>` | — |
| `FnSpout<F>` | Calls a closure for each item | — |
| `FnFlushSpout<S, F>` | Separate closures for send and flush | — |
| `BatchSpout<T, S>` | Buffers items, forwards as `Vec<T>` batches | — |
| `ReduceSpout<T, R, F, S>` | Batches, reduces, then forwards the result | — |
| `ProducerSpout<S, F>` | Multi-producer with unique IDs per clone | — |
| `SequencedSpout<T, Inner>` | Reorders out-of-order batches by sequence number | — |
| `ChannelSpout<T>` | Wraps `mpsc::Sender<T>` | `std` |
| `SyncChannelSpout<T>` | Wraps `mpsc::SyncSender<T>` with blocking backpressure | `std` |
| `Arc<Mutex<S>>` | Thread-safe shared spout via mutex | `std` |
| `FramedSpout<S>` | Serializes items with framing headers (producer ID + length) | `bytecast` |

## Traits

| Trait | Description |
|-------|-------------|
| `Spout<T>` | Core push trait — `send()`, `send_all()`, `flush()` |
| `Flush` | Flush behavior abstraction for composing with closures |

## Feature Flags

| Feature    | Description |
|------------|-------------|
| `std`      | Enables `ChannelSpout`, `SyncChannelSpout`, and `Arc<Mutex<S>>` support |
| `bytecast` | Enables `FramedSpout`, frame encoding/decoding, and `BatchSpout` serialization |

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.
