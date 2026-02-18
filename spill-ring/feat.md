# Feature Roadmap: TPC Emergent Patterns

Now that the TPC WorkerPool is in place (per-worker `HandoffSlot` + `WorkerSignal`, pre-allocated ring rotation, zero-allocation steady state), the following features emerge naturally. Each builds on the previous — the sequence matters.

---

## Feature 1: Streaming Collection API (`dispatch` / `collect` / `join`)

### Problem

`run()` is synchronous: signal workers, block until all done, return. No way to collect results while workers are still running. The handoff slots already support concurrent read (consumer) + write (producer), but the API doesn't expose it.

### Design

Split `run()` into three phases:

```rust
impl WorkerPool<T, N, S, F, A> {
    /// Non-blocking: signal all workers to start. Returns immediately.
    pub fn dispatch(&mut self, args: &A);

    /// Collect all available batches from handoff slots into a spout.
    /// Can be called while workers are still running.
    /// Returns the number of items drained.
    pub fn collect<Out: Spout<T>>(&mut self, out: &mut Out) -> usize;

    /// Block until all workers signal done. Detects panics.
    pub fn join(&mut self) -> Result<(), WorkerPanic>;

    /// Synchronous: dispatch + join. Existing API, unchanged.
    pub fn run(&mut self, args: &A);
}
```

`collect()` iterates all handoff slots, swaps null into `batch` (Acquire), drains the ring into the output spout, then offers the empty ring back into `recycle` (Release). Workers see the recycled ring on their next publish cycle. The consumer never blocks — if a slot is empty (worker hasn't published yet), it's skipped.

### Key Details

- `collect()` takes `&mut self` — only one collector at a time (MPSC invariant).
- `run()` becomes `dispatch(); join();` internally (backward compatible).
- `collect()` can be called between `dispatch()` and `join()`, or after `join()`, or between `run()` calls.
- Ring recycling in `collect()` closes the triple-buffer loop: active → published → recycled → active.
- The `into_consumer()` path still works — it calls `shutdown_and_join()` which collects everything.

### Files

| File | Change |
|------|--------|
| `mpsc/pool.rs` | Add `dispatch()`, `collect()`, `join()`. Refactor `try_run()` to use them. |
| `tests/mpsc.rs` | Add streaming collection tests: collect during dispatch, interleaved collect+run. |

### Dependencies

None — builds directly on the existing TPC implementation.

---

## Feature 2: `FanInSpout` — Composable N-to-1 Collection

### Problem

`pool.collect(&mut sink)` couples collection to the pool. To compose TPC output with downstream stages (batching, framing, network I/O), collection should be a spout itself — a source that drains handoff slots and emits items to its inner spout.

### Design

```rust
/// Collects from N handoff slots, emits to downstream spout.
///
/// Implements `Spout<T>` on the output side. On the input side,
/// `flush()` triggers collection from all handoff slots.
///
/// Usage: pool.dispatch(&args) → fan_in.flush() → items flow to inner spout.
pub struct FanInSpout<T, const N: usize, S, Out> {
    /// Raw pointers to handoff slots owned by WorkerPool.
    slots: Box<[SendPtr<HandoffSlot<T, N, S>>]>,
    /// Downstream spout receiving collected items.
    inner: Out,
}

impl<T, const N: usize, S, Out: Spout<T>> Spout<T> for FanInSpout<T, N, S, Out> {
    type Error = Out::Error;

    /// Pass-through: forward an externally-produced item to inner.
    fn send(&mut self, item: T) -> Result<(), Self::Error> {
        self.inner.send(item)
    }

    /// Collect all available batches from handoff slots, drain into inner.
    fn flush(&mut self) -> Result<(), Self::Error> {
        for slot_ptr in self.slots.iter() {
            let slot = unsafe { &*slot_ptr.0 };
            if let Some(mut ring) = slot.collect() {
                self.inner.send_all(ring.drain())?;
                slot.offer_recycle(ring);
            }
        }
        self.inner.flush()
    }
}
```

This makes TPC collection composable: `WorkerPool → FanInSpout<_, _, _, FramedSpout<TcpStream>>`. Any downstream that accepts `Spout<T>` can consume from a TPC pool without knowing it's parallel underneath.

### Construction

`FanInSpout` is constructed from `WorkerPool` by borrowing pointers to its handoff slots. Lifetime safety: the `FanInSpout` must not outlive the pool. This can be enforced via:

- A method on `WorkerPool` that returns `FanInSpout` with a borrow: `pool.fan_in(&mut sink) -> FanInSpout<'_, ...>`
- Or an unsafe constructor with a `SendPtr` + documented lifetime contract (same pattern as worker threads).

The borrow approach is safer but limits usage patterns. Start with the borrow approach; relax to unsafe if needed.

### Files

| File | Change |
|------|--------|
| `mpsc/fan_in.rs` | New file: `FanInSpout` implementation. |
| `mpsc/mod.rs` | Add `mod fan_in;`, re-export `FanInSpout`. |
| `lib.rs` | Re-export `FanInSpout`. |
| `tests/mpsc.rs` | Tests: fan_in with CollectSpout, fan_in with BatchSpout, fan_in composition. |

### Dependencies

Feature 1 (`dispatch`/`collect`/`join`) — `FanInSpout` is the spout-interface equivalent of `collect()`.

---

## Feature 3: `SequencedSpout` — Ordered Completion

### Problem

Workers complete in arbitrary order. Per-producer FIFO is guaranteed by the ring, but cross-producer ordering is nondeterministic. Some workloads require global ordering — batches must be emitted to the downstream spout in the same sequence they were dispatched.

### Design

Each `dispatch()` call increments a batch sequence counter (free — plain `u64` on the main thread). Workers stamp their published ring with the batch sequence number. `SequencedSpout` buffers out-of-order batches and emits in strict sequence:

```rust
/// Reorders batches by sequence number before forwarding to inner spout.
/// Fast path: if the next expected sequence arrives, emit directly (zero buffering).
/// Slow path: buffer until predecessors arrive, then flush in order.
pub struct SequencedSpout<T, Inner: Spout<T>> {
    next_expected: u64,
    pending: BTreeMap<u64, Vec<T>>,
    inner: Inner,
}

impl<T, Inner: Spout<T>> SequencedSpout<T, Inner> {
    /// Submit a batch with its sequence number.
    pub fn submit(&mut self, batch_seq: u64, items: impl Iterator<Item = T>)
        -> Result<(), Inner::Error>
    {
        if batch_seq == self.next_expected {
            // Fast path: in order, emit directly.
            for item in items {
                self.inner.send(item)?;
            }
            self.next_expected += 1;
            // Drain any buffered batches that are now in order.
            while let Some(buffered) = self.pending.remove(&self.next_expected) {
                for item in buffered {
                    self.inner.send(item)?;
                }
                self.next_expected += 1;
            }
        } else {
            // Out of order: buffer until predecessors arrive.
            self.pending.insert(batch_seq, items.collect());
        }
        Ok(())
    }
}
```

This is io_uring's completion queue pattern — parallel submission, ordered completion — generalized through the spout trait.

### Batch Sequence Stamping

The batch sequence needs to be associated with each handoff slot's published ring. Options:

1. **Add a `batch_seq` field to `HandoffSlot`** — one extra `AtomicU64` per slot, set by main thread before signaling go.
2. **Wrap the ring pointer with metadata** — publish `(seq, Box<SpillRing>)` instead of just the pointer.

Option 1 is simpler and avoids changing the pointer type. The main thread stores the sequence into each slot's `batch_seq` before calling `dispatch()`. The consumer reads it alongside `collect()`.

### Where It Lives

`SequencedSpout` is a generic spout combinator — it doesn't depend on spill-ring internals. It belongs in the **spout** crate, not spill-ring. It composes with any spout.

### Files

| File | Change |
|------|--------|
| `spout/src/impls/core_impls.rs` | Add `SequencedSpout` implementation. |
| `spout/src/lib.rs` | Re-export `SequencedSpout`. |
| `spill-ring/src/mpsc/handoff.rs` | Add `batch_seq: AtomicU64` to `HandoffSlot`. |
| `spill-ring/src/mpsc/pool.rs` | Increment batch counter in `dispatch()`, store into slots. |
| `spill-ring/src/mpsc/fan_in.rs` | Update `FanInSpout::flush()` to pass batch_seq to downstream. |
| `spout/src/tests/` | Tests: in-order fast path, out-of-order buffering, mixed. |
| `spill-ring/src/tests/mpsc.rs` | Integration test: pool → fan_in → sequenced → collect. |

### Dependencies

Feature 1 (batch dispatch counter), Feature 2 (FanInSpout to feed batches with seq numbers).

---

## Feature 4: Backpressure Propagation (Documentation + Validation)

### Problem

The TPC design has structural backpressure built in — when the downstream spout is slow, handoff slots fill up, workers detect uncollected batches, merge them back, rings fill to capacity, overflow spouts fire. But this cascade is undocumented and untested.

### Design

This is primarily a **documentation and testing** feature, not new code. The backpressure chain:

```
Output spout slow
  → consumer collect() slows (fewer calls, or calls take longer)
    → handoff slots stay full (batch_ptr non-null when worker tries to publish)
      → worker detects uncollected batch, merges back into active ring
        → ring fills to capacity
          → overflow spout fires (eviction / backpressure callback)
```

Each layer already has its valve:

| Layer | Mechanism | Already Implemented? |
|-------|-----------|---------------------|
| SpillRing overflow → spout | Eviction on push when full | Yes |
| HandoffSlot → merge back | Worker merges uncollected batch | Yes |
| Consumer → drain rate | `collect()` frequency | Yes (with Feature 1) |
| Output spout → I/O throughput | External | N/A |

### Work

1. **Document** the backpressure chain in the module-level docs of `mpsc/mod.rs`.
2. **Add a backpressure test**: use a `SlowSpout` (inserts a sleep in `send()`) as the output. Verify that when the consumer is slow, items flow to the overflow spout rather than being lost.
3. **Add a `BackpressureSpout`** adapter (optional): a spout wrapper that counts dropped/evicted items and exposes backpressure metrics. This is useful for observability.

### Files

| File | Change |
|------|--------|
| `mpsc/mod.rs` | Add backpressure documentation to module docs. |
| `tests/mpsc.rs` | Add backpressure test with slow consumer. |
| `spout/src/impls/core_impls.rs` | (Optional) `CountingSpout` wrapper for backpressure metrics. |

### Dependencies

Feature 1 (streaming `collect()` to observe backpressure in real-time).

---

## Feature 5: Multi-Stage Parallel Pipelines

### Problem

Real workloads are multi-stage: parse → transform → serialize → I/O. Each stage should independently scale to its core allocation. The spout trait and TPC handoff make this composable, but the composition pattern needs to be validated and documented.

### Design

A pipeline is just nested type composition:

```
Stage 1: WorkerPool<Raw, N1, FanInSpout<_, _, _, WorkerPool<Parsed, N2, ...>>>
                                                    ↓
Stage 2: WorkerPool<Parsed, N2, FanInSpout<_, _, _, SequencedSpout<_, FramedSpout<...>>>>
                                                    ↓
         FramedSpout<TcpStream>
```

Each stage is a `WorkerPool` whose output `FanInSpout` feeds the next stage's input. The compiler monomorphizes the entire chain — zero dynamic dispatch.

### Key Insight

`FanInSpout` from Feature 2 implements `Spout<T>`, so it can be the overflow sink for the next stage's `SpillRing`. The pattern:

```rust
// Stage 1: parse raw data on 4 cores
let mut stage1 = MpscRing::<Parsed, 1024>::pool(4)
    .spawn(|ring, _id, raw: &[u8]| {
        // parse raw into ring
    });

// Stage 2: transform on 2 cores, output to framed spout
let mut stage2 = MpscRing::<Enriched, 512>::pool(2)
    .spawn(|ring, _id, batch: &Vec<Parsed>| {
        // transform batch into ring
    });

// Run pipeline:
stage1.dispatch(&raw_data);
stage1.join()?;
// Collect stage 1 output, feed to stage 2
let mut stage1_output = Vec::new();
stage1.collect(&mut CollectSpout::from(&mut stage1_output));
stage2.dispatch(&stage1_output);
stage2.join()?;
stage2.collect(&mut framed_output);
```

### Work

This is primarily an **integration test and example**, not new library code. The composition already works through existing types. What's needed:

1. **Example**: `examples/pipeline.rs` — two-stage parallel pipeline demonstrating composition.
2. **Test**: Multi-stage pipeline test that verifies data integrity across stages.
3. **Documentation**: Module-level docs showing the pipeline pattern.

### Files

| File | Change |
|------|--------|
| `examples/pipeline.rs` | New example: two-stage parallel pipeline. |
| `Cargo.toml` | Add `[[example]]` entry with `required-features = ["std"]`. |
| `tests/mpsc.rs` | Integration test: two-stage pipeline, verify all items arrive. |
| `mpsc/mod.rs` | Pipeline pattern in module docs. |

### Dependencies

Feature 1 (dispatch/collect/join), Feature 2 (FanInSpout).

---

## Feature 6: MPSM — Multiple Producers, Multiple Mergers

### Problem

MPSC (single consumer) limits collection throughput to one core. For high fan-in (many producers), the consumer becomes the bottleneck. MPSM partitions handoff slots across M merger threads, each draining a subset of producers.

### Design

The handoff slot design makes this natural — each slot is independent. Assigning different slots to different mergers requires no coordination with producers. The producer doesn't know or care who reads its slot.

```rust
/// A merger that owns a subset of handoff slots.
/// Each merger runs on its own thread, draining its assigned slots
/// into a local FanInSpout.
pub struct MergerHandle<T, const N: usize, S, Out> {
    fan_in: FanInSpout<T, N, S, Out>,
}

impl WorkerPool<T, N, S, F, A> {
    /// Partition handoff slots across M mergers.
    /// Returns M MergerHandles, each owning N/M slots.
    pub fn split_mergers<Out: Spout<T>>(
        &self,
        num_mergers: usize,
        sink_factory: impl Fn() -> Out,
    ) -> Vec<MergerHandle<T, N, S, Out>>;
}
```

### Merge Tree

For very high fan-in, mergers form a tree:

```
Producers:  P0  P1  P2  P3  P4  P5  P6  P7
              \  |  /     \  |  /     \  |
Mergers L1: FanIn[0..2]  FanIn[3..5] FanIn[6..7]
              │              │             │
Mergers L2:     FanIn[merger_0..merger_2]
                      │
               SequencedSpout
                      │
                  I/O Spout
```

Each merge tier is the same type composition: `FanInSpout` → optional `SequencedSpout` → output spout. The tree scales logarithmically.

### Work

1. `MergerHandle` type that owns a `FanInSpout` over a subset of slots.
2. `WorkerPool::split_mergers()` method that partitions slots round-robin.
3. Each merger runs on its own thread (spawned by the user, not by spill-ring — consistent with TPC philosophy).
4. Integration test: 8 producers, 2 mergers, verify all items collected.

### Files

| File | Change |
|------|--------|
| `mpsc/merger.rs` | New file: `MergerHandle` type. |
| `mpsc/pool.rs` | Add `split_mergers()` method. |
| `mpsc/mod.rs` | Add `mod merger;`, re-export `MergerHandle`. |
| `lib.rs` | Re-export `MergerHandle`. |
| `tests/mpsc.rs` | MPSM integration test. |

### Dependencies

Feature 2 (FanInSpout — mergers are FanInSpouts over slot subsets).

---

## Implementation Sequence

```
Feature 1: Streaming Collection API
    │
    ├──> Feature 2: FanInSpout
    │        │
    │        ├──> Feature 3: SequencedSpout
    │        │
    │        ├──> Feature 5: Multi-Stage Pipelines (example + test)
    │        │
    │        └──> Feature 6: MPSM
    │
    └──> Feature 4: Backpressure (docs + test)
```

Feature 1 is the foundation — everything else builds on `dispatch`/`collect`/`join`. Feature 2 wraps collection as a spout. Features 3-6 compose from there.

---

## Design Philosophy

From tpc.md:

> No `Pipeline` struct. No `Stage` trait. No runtime graph. The types compose, the user builds the graph, the compiler inlines and monomorphizes. Zero-cost abstraction — the assembled pipeline compiles to the same code as a hand-written version.

Every feature above is a building block, not a framework. `FanInSpout` is a spout. `SequencedSpout` is a spout. `MergerHandle` wraps a `FanInSpout`. The user assembles them. The compiler optimizes them.
