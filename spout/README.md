# spout

A minimal, composable push-based trait for sending items to a destination. The inverse of iterators — push instead of pull.

## Features

- **no_std by default** - Works on embedded devices with just `alloc`
- **Zero dependencies** - Pure Rust, no external crates
- **Composable** - Spouts wrap spouts for batching, reduction, and transformation
- **Simple API** - Just `send()`, `send_all()`, and `flush()`

## Usage

Add to your `Cargo.toml`:

```toml
[dependencies]
spout = "1.0.0-rc.1"
```

### Basic Example

```rust
use spout::{CollectSpout, Spout};

let mut s = CollectSpout::new();
s.send(1);
s.send(2);
s.send(3);
assert_eq!(s.items(), &[1, 2, 3]);
```

### Closure-Based Spout

```rust
use spout::{FnSpout, Spout};

let mut collected = Vec::new();
let mut s = FnSpout(|x: i32| collected.push(x));
s.send(10);
s.send(20);
```

### Batching

Buffer items and forward in batches:

```rust
use spout::{BatchSpout, CollectSpout, Spout};

let mut s = BatchSpout::new(3, CollectSpout::new());
s.send(1);
s.send(2);
s.send(3); // triggers batch forward
assert_eq!(s.inner().items(), &[vec![1, 2, 3]]);
```

### Reduction

Batch and transform before forwarding:

```rust
use spout::{ReduceSpout, CollectSpout, Spout};

let mut s = ReduceSpout::new(
    4,
    |batch: Vec<i32>| batch.iter().sum::<i32>(),
    CollectSpout::new(),
);

for i in 1..=8 {
    s.send(i);
}
s.flush();
assert_eq!(s.into_inner().into_items(), vec![10, 26]);
```

### Multi-Producer

Each clone gets a unique ID and independent spout:

```rust
use spout::{ProducerSpout, CollectSpout, Spout};

let s = ProducerSpout::new(|_id| CollectSpout::<i32>::new());
let mut s0 = s.clone();
let mut s1 = s.clone();

s0.send(1);
s1.send(2);
```

## Feature Flags

| Feature    | Description |
|------------|-------------|
| `std`      | Enables `ChannelSpout` and `Arc<Mutex<S>>` support |
| `bytecast` | Enables `FramedSpout`, frame encoding/decoding, and `BatchSpout` serialization |

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.
