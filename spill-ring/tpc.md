# Thread-Per-Core WorkerPool Redesign

## Problem Statement

The current `WorkerPool` uses a `SpinBarrier` for synchronization, which introduces:

1. **Deadlock on worker panic** — if any worker panics, the remaining workers and the main thread spin forever on the barrier (security.md H-1).
2. **O(N) synchronization per batch** — the start and done barriers require all N workers + main to arrive, totaling 2*(N+1) synchronization points per batch. For short work units, this dominates latency.
3. **Batch-only execution** — all workers must be dispatched together and complete together before results can be consumed. No streaming.
4. **Unordered collection** — `Consumer::drain` iterates rings sequentially by producer index. No global ordering across producers.
5. **No MPSM path** — the consumer is inherently single. No way to partition collection across multiple merger threads.

The produce path is already contention-free (~1.8 Gelem/s per worker via `Cell`-based `SpillRing::push`). The redesign targets the synchronization and collection layers while preserving this property.

---

## Design Goals

1. **Thread-per-core**: each worker owns its ring with ZERO contention on the produce path.
2. **MPSC collection**: multiple producers, single consumer — collection must be extremely cheap.
3. **MPSM extensibility**: multiple producers, multiple mergers — falls out naturally from MPSC.
4. **Correct ordering**: per-producer FIFO guaranteed; global ordering opt-in.
5. **Contention eliminated or minimized**: contention exists ONLY at batch boundaries, not per-item.

---

## Research: Probabilistic Data Structures

### Candidates Evaluated

| Structure | Purpose | Applicable? | Reason |
|-----------|---------|-------------|--------|
| HyperLogLog | Cardinality estimation | No | Answers "how many distinct items?" — cannot reconstruct items |
| Count-Min Sketch | Frequency estimation | No | Answers "how often did X appear?" — loses items themselves |
| Bloom Filter | Set membership | No | Answers "was X seen?" with false positives — false positives mean data loss |
| Probabilistic Skip List | Ordered insertion | Marginal | Exact structure (not probabilistic in the sketch sense), O(log N) insert, contention on shared structure — strictly worse than ring buffer |
| T-Digest / Q-Digest | Quantile estimation | No | Answers "what is the Nth percentile?" — completely inapplicable to ordered collection |

### Assessment

The collection problem requires **exact, ordered handoff** of items from N producers to 1 (or M) consumers. Probabilistic structures trade exactness for space/time savings. You cannot afford to lose items or approximate their order.

The one potential use — estimating "how much data is ready across all producers" without reading each state — is trivially solved by having each producer publish its tail index atomically, which is exact and costs the same as an approximate approach.

**Probabilistic data structures do not help here.**

---

## Research: Lock-Free / Wait-Free Collection Patterns

### Per-Producer Sequence Numbers + Merge-Sort Consumer

Each producer stamps items with a monotonic counter. The consumer performs a k-way merge.

**Sequence number assignment options:**

| Method | Produce Cost | Ordering Guarantee | Verdict |
|--------|-------------|-------------------|---------|
| Global `AtomicU64::fetch_add` | ~20ns/item under contention | Total order | **Rejected** — at 1.8 Gelem/s (0.55ns/item), a 20ns atomic reduces throughput to ~50 Melem/s |
| Per-producer local counter | Zero (plain u64 increment) | Partial order (total within producer) | **Recommended** |
| Lamport clock | Zero (degenerates to local counter when producers are independent) | Causal order | Equivalent to local counter here |
| Vector clock | O(N) per event | Full causal order | Overkill — producers don't communicate, no causality to track |

**Key insight**: items produced by different workers are **causally independent**. There is no meaningful "correct" global ordering — only per-producer ordering matters. Any total order imposed is arbitrary. The consumer should provide per-producer FIFO guarantees and let the application define the merge policy.

### Epoch-Based Handoff (Double-Buffering)

Producers accumulate items during epoch E. When the consumer wants to collect, it advances to epoch E+1. Each producer publishes its epoch-E data (by swapping its ring) and begins accumulating into epoch E+1 buffers. The consumer reads all epoch-E data safely without contention.

This is the **double-buffering** pattern. **Recommended as the core mechanism.**

### Flat Combining

One thread collects pending operations from all threads and executes them serially. Designed for cases where operations must be serialized (concurrent stack/queue). Here, produce is already contention-free and collection is inherently serial. Flat combining adds overhead to publication without benefit. **Not a good fit.**

### Elimination Stacks/Arrays

Pairs of push/pop operations "eliminate" each other without touching the shared structure. Requires concurrent push and pop arriving simultaneously. In spill-ring, produce and consume are separated in time. **Not applicable.**

### LCRQ (Linked Concurrent Ring Queue)

Lock-free MPMC queue built from linked ring segments. Wait-free enqueue, lock-free dequeue. Could replace per-producer rings with a single shared LCRQ, but this introduces contention on every push — sacrificing the ~1.8 Gelem/s per-worker throughput. **Rejected as primary approach.**

---

## Research: Thread-Per-Core Architectures in Practice

### Seastar / ScyllaDB

Each core runs a single thread ("shard"). Shards own their data exclusively. Cross-shard communication uses per-pair SPSC queues. Collection is done by the requesting shard: it sends requests to each shard, each responds, the requester collects.

**Takeaway**: Per-pair SPSC queues. No shared data structures. The collector explicitly pulls from each producer's queue.

### DPDK rte_ring

Supports MPSC mode with CAS-based producer coordination. Contention is on the producer side (multiple producers contend on shared head/tail). **Opposite of what spill-ring wants.** The relevant lesson: for MPSC with zero producer contention, use per-producer rings with consumer-side polling.

### io_uring Submission/Completion Queues

Two SPSC rings with atomic head/tail in shared memory. Producer writes entries and advances tail. Consumer reads entries and advances head. **Gold standard for single-producer, single-consumer handoff.** Each producer could maintain an SPSC ring to the consumer.

### Glommio / Monoio

Follow Seastar's model: thread-per-core, no work stealing, explicit cross-shard channels. `SharedChannel` is SPSC under the hood with batched transfers and eventfd for wake-up. **Validates the SPSC-per-pair + eventfd-wake pattern for Rust.**

---

## Recommended Architecture: Double-Buffered Swap with Per-Worker Signaling

### Overview

Replace the barrier-synchronized batch model with a **double-buffered swap** model. Each producer accumulates into a local ring and publishes completed batches to the consumer via a lock-free handoff slot.

```
Producer 0: [Ring] ──swap──> [Handoff Slot 0] ──read──> Consumer
Producer 1: [Ring] ──swap──> [Handoff Slot 1] ──read──> Consumer
Producer 2: [Ring] ──swap──> [Handoff Slot 2] ──read──> Consumer
                                                          |
                                                     [Merge Output]
```

### Data Flow

**Produce path (ZERO contention):**

```
1. Producer calls push(item):
   - active_ring.push(item)     // Cell-based, no atomics. O(1).
   - local_seq += 1             // Plain u64 increment.

   Identical to current SpillRing::push() hot path.
   No atomics. No shared state. No cache-line bouncing.
   Expected throughput: ~1.8 Gelem/s per worker (unchanged).
```

**Publish path (once per batch, amortized):**

```
2. When batch is complete:
   a. Producer wraps active ring in Box, converts to raw pointer.
   b. AtomicPtr::swap(ptr, Release) into handoff slot.
   c. Producer checks recycle slot for an empty ring from consumer.
      - If available: take it (zero allocation).
      - If not: allocate a fresh ring.
   d. If previous batch was uncollected (old ptr non-null):
      merge old batch into current ring (backpressure).

   Cost: One atomic store (Release) per batch.
   Amortized over thousands of items.
```

**Consume path (single consumer, can stream):**

```
3. Consumer calls collect():
   a. For each slot i in 0..num_producers:
      - AtomicPtr::swap(null, Acquire) on handoff slot.
      - If non-null: consumer now owns the ring exclusively.
        Drain items. Recycle empty ring back to producer.
      - If null: producer hasn't published yet. Skip.

   b. Per-producer FIFO ordering is guaranteed by SpillRing.
      Cross-producer ordering: deterministic by producer index,
      or by batch sequence number if temporal ordering is needed.

   Cost: N atomic loads (one per producer) + O(total_items) drain.
```

### Per-Worker Signaling (Replaces SpinBarrier)

Replace the single barrier (N+1 participants) with N independent signal structs:

```
  Main thread         Worker 0           Worker 1           Worker N
  ----------         --------           --------           --------
  store args_ptr
  go[0] = true        [spinning/parked]
  go[1] = true                           [spinning/parked]
  go[N] = true                                              [spinning/parked]
                      wake, read args     wake, read args    wake, read args
                      work(&ring, ...)    work(&ring, ...)   work(&ring, ...)
                      publish ring        publish ring        publish ring
                      done[0] = true      done[1] = true     done[N] = true
  poll done signals
  [all true? proceed]
```

**Why this is better than a barrier:**

1. **No deadlock on panic.** If worker K panics, `done[K]` is never set. Main thread detects via `JoinHandle::is_finished()` and handles gracefully.
2. **No O(N) synchronization.** Each worker only touches its own signals. No shared counter. No generation.
3. **Main thread can poll progressively.** Process results from fast workers while slow workers are still running.
4. **Easy MPSM extension.** Multiple consumer threads each poll a subset of done signals.

### Wake-Up Strategy

**Adaptive spin-then-park** (recommended):

- Spin for a configurable window (matching current `spin_limit` logic).
- Fall back to `thread::yield_now()` after spin limit.
- For truly long waits, use `thread::park()` / `thread::unpark()`.
- Zero syscalls in the fast path (work completes within spin window).
- Graceful degradation for long batches.
- Fully portable (no Linux-specific APIs).

For a future event-driven (non-batch) model, `eventfd` (Linux) or `kqueue` (macOS) would be more appropriate.

### Triple-Buffer Ring Reuse

Each producer has three ring states cycling through the system:

1. **Active**: currently being written to by the producer.
2. **Published**: handed off to consumer, consumer is reading it.
3. **Recycled**: consumer finished reading, returned to producer.

```
Producer ──[active ring]──swap──> Handoff Slot ──swap──> Consumer
    ^                                                        |
    |                                                        |
    +────────────[recycled ring]──swap──> Recycle Slot ──────+
```

Zero-allocation steady state after warmup. Rings cycle between active, published, and recycled states. Each transition is a single atomic pointer swap.

---

## Contention Analysis

| Operation | Contention | Cross-Thread Cache Lines | Frequency |
|-----------|-----------|------------------------|-----------|
| `push(item)` | **NONE** | 0 | Per item |
| `local_seq += 1` | **NONE** | 0 | Per item |
| `publish()` — producer | Minimal — 1 atomic swap | 1 (handoff slot) | Per batch |
| `collect()` — consumer | Minimal — N atomic loads | N (one per slot) | Per collection round |
| `go[i] = true` — main | **NONE** (per-worker) | 1 | Per batch |
| `done[i] = true` — worker | **NONE** (per-worker) | 1 | Per batch |
| `drain(ring)` — consumer | **NONE** | 0 (exclusive ownership) | Per collection round |

**Total contention on produce path: ZERO.**
**Total contention on publish: ONE atomic store, amortized over entire batch.**
**Total contention on consume: N atomic loads, one per producer.**

Compare with current design:
- Produce path: Zero (same).
- Barrier sync per batch: O(N) atomic operations on shared counter + spin-waiting. Two barriers per batch (start + done) = 2 * O(N). Deadlock-prone.

---

## Ordering Guarantees

| Guarantee | Produce Cost | Consume Cost | Mechanism |
|-----------|-------------|-------------|-----------|
| Per-producer FIFO | Zero | Zero | Inherent in ring buffer |
| Deterministic interleave | Zero | O(P) sort by worker_id | Batches tagged with worker_id |
| Batch-level temporal ordering | Zero | O(B * log P) k-way merge | Per-producer batch counter (free) |
| Global total order (opt-in) | ~20ns/item (atomic increment) | O(items * log P) k-way merge | Global `AtomicU64` sequence counter |

**Default**: per-producer FIFO with deterministic interleave by producer index. The batch-level sequence counter is free (plain u64 increment per batch) and provides temporal ordering of batches if needed.

Global total ordering is opt-in because it costs ~20ns per item on the produce path — a 36x slowdown from 1.8 Gelem/s to 50 Melem/s. Most workloads don't need it.

---

## MPSM Extension

MPSM (Multiple Producers, Single Merger → Multiple Mergers) falls out naturally from the per-worker handoff slot design.

### MPSC (baseline)

One consumer polls all N handoff slots.

### MPSM with M mergers

Partition the N handoff slots into M groups. Each merger owns N/M slots and runs on its own core.

```
Producers:  P0  P1  P2  P3  P4  P5  P6  P7
              \  |  /     \  |  /     \  |
Mergers:    Merger 0     Merger 1     Merger 2
                \           |           /
                  \         |         /
              Final Consumer (or another merge tier)
```

**Why this works with zero changes to the producer side**: each handoff slot is independent. Assigning different slots to different mergers requires no coordination with producers. The producer does not know or care who reads its slot.

Each merger writes to its own output ring or spout, forming a merge tree that scales logarithmically with the number of producers.

---

## Proposed Type Hierarchy

### Core Types

```rust
/// Per-producer handoff slot. Cache-line aligned to prevent false sharing.
/// Producer writes batch_ptr. Consumer reads batch_ptr.
/// Consumer writes recycle_ptr. Producer reads recycle_ptr.
#[repr(C, align(128))]
pub(crate) struct HandoffSlot<T, const N: usize, S> {
    /// Published ring, or null. Producer -> Consumer.
    batch: AtomicPtr<SpillRing<T, N, S>>,
    /// Recycled empty ring, or null. Consumer -> Producer.
    recycle: AtomicPtr<SpillRing<T, N, S>>,
}

/// Per-worker control signals. Cache-line aligned.
#[repr(C, align(128))]
pub(crate) struct WorkerSignal {
    /// Main -> Worker: "start working". Cleared by worker after reading.
    go: AtomicBool,
    /// Worker -> Main: "work complete, batch published".
    done: AtomicBool,
    /// Main -> Worker: "shut down after next wake".
    shutdown: AtomicBool,
}
```

### WorkerPool v2

```rust
pub struct WorkerPool<T, const N: usize, S, F, A> {
    num_workers: usize,
    handles: Vec<Option<JoinHandle<()>>>,
    /// Stable allocation of handoff slots. Shared with workers via raw pointer.
    slots: Pin<Box<[HandoffSlot<T, N, S>]>>,
    /// Stable allocation of worker signals.
    signals: Pin<Box<[WorkerSignal]>>,
    /// Args pointer for current batch.
    args_ptr: AtomicPtr<A>,
    _marker: PhantomData<F>,
}

impl<T, const N, S, F, A> WorkerPool<T, N, S, F, A> {
    /// Dispatch work to all workers. Non-blocking.
    pub fn dispatch(&mut self, args: &A) {
        self.args_ptr.store(args as *const A as *mut A, Ordering::Release);
        for sig in self.signals.iter() {
            sig.go.store(true, Ordering::Release);
        }
    }

    /// Wait for all workers to complete. Detects panics gracefully.
    pub fn join(&mut self) -> Result<(), WorkerPanic> {
        // Adaptive spin-then-yield polling done signals.
        // Check JoinHandle::is_finished() to detect panicked workers.
    }

    /// Synchronous run: dispatch + join. Backward-compatible API.
    pub fn run(&mut self, args: &A) -> Result<(), WorkerPanic> {
        self.dispatch(args);
        self.join()
    }

    /// Collect results from all workers that have published.
    /// Can be called while workers are still running (streaming).
    pub fn collect<Out: Spout<T>>(&self, out: &mut Out) {
        for slot in self.slots.iter() {
            let ptr = slot.batch.swap(core::ptr::null_mut(), Ordering::Acquire);
            if !ptr.is_null() {
                let mut ring = unsafe { *Box::from_raw(ptr) };
                let _ = out.send_all(ring.drain());
                // Recycle ring back to producer
                let recycle_ptr = Box::into_raw(Box::new(ring));
                let old = slot.recycle.swap(recycle_ptr, Ordering::Release);
                if !old.is_null() {
                    drop(unsafe { Box::from_raw(old) });
                }
            }
        }
    }
}
```

### Worker Loop v2

```rust
fn worker_loop<T, const N: usize, S, F, A>(
    mut ring: SpillRing<T, N, S>,
    worker_id: usize,
    work: F,
    signal: &WorkerSignal,
    slot: &HandoffSlot<T, N, S>,
    args_ptr: &AtomicPtr<A>,
    sink_factory: impl Fn() -> S,
) {
    loop {
        // Adaptive spin-then-yield on go signal
        wait_for_go(signal);

        if signal.shutdown.load(Ordering::Relaxed) {
            // Publish remaining ring contents before exit
            let ptr = Box::into_raw(Box::new(ring));
            slot.batch.store(ptr, Ordering::Release);
            return;
        }
        signal.go.store(false, Ordering::Relaxed);

        // Execute work
        let args = unsafe { &*args_ptr.load(Ordering::Acquire) };
        work(&ring, worker_id, args);

        // Try to get a recycled ring from consumer, or allocate fresh
        let fresh = {
            let recycled = slot.recycle.swap(core::ptr::null_mut(), Ordering::Acquire);
            if recycled.is_null() {
                SpillRing::with_sink(sink_factory())
            } else {
                unsafe { *Box::from_raw(recycled) }
            }
        };

        // Swap active ring into handoff slot
        let published = core::mem::replace(&mut ring, fresh);
        let ptr = Box::into_raw(Box::new(published));
        let old = slot.batch.swap(ptr, Ordering::Release);

        // If consumer hasn't collected previous batch, merge it back
        if !old.is_null() {
            let old_ring = unsafe { *Box::from_raw(old) };
            for item in old_ring.drain() {
                ring.push_mut(item);
            }
        }

        // Signal done
        signal.done.store(true, Ordering::Release);
    }
}
```

### Consumer Handle

```rust
pub struct PoolConsumer<T, const N: usize, S> {
    slots: Vec<*const HandoffSlot<T, N, S>>,
}

impl<T, const N: usize, S> PoolConsumer<T, N, S> {
    /// Collect all available batches. Returns owned rings.
    pub fn collect_batches(&mut self) -> Vec<SpillRing<T, N, S>> { ... }

    /// Drain all collected batches into a spout.
    pub fn drain_into<Out: Spout<T>>(&mut self, out: &mut Out) { ... }
}

/// MPSM: partition slots across multiple mergers.
pub struct MergerHandle<T, const N: usize, S> {
    assigned_slots: Vec<*const HandoffSlot<T, N, S>>,
}
```

---

## Alternatives Considered and Rejected

### Shared MPSC Ring (LCRQ-style)

All producers push to a single shared ring. Consumer pops from it.

**Rejected**: introduces contention on every push. Per-worker throughput drops from ~1.8 Gelem/s to ~200 Melem/s under contention. Defeats the entire design philosophy.

### Crossbeam Channels (per producer)

Each producer sends items through a crossbeam MPSC channel to the consumer.

**Rejected**: per-item overhead (allocation, atomic operations). At 1.8 Gelem/s, you cannot afford any per-item cross-thread coordination. Channels are for large, infrequent items — not billions of small ones.

### Concurrent Skip List

All producers insert into a shared concurrent skip list ordered by sequence number.

**Rejected**: O(log N) per insert vs O(1) for ring buffer. Contention on shared structure. Memory allocation per insert. Orders of magnitude slower.

### Flat Combining

One thread collects operations from all threads and executes them serially.

**Rejected**: designed for cases where operations must be serialized. Here, produce is already contention-free and collection is inherently serial. Adds overhead without benefit.

### Elimination Stacks/Arrays

Pairs of push/pop operations eliminate each other without touching the shared structure.

**Rejected**: requires concurrent push and pop arriving simultaneously. In spill-ring, produce and consume are separated in time.

### Keep Barriers, Fix Panic Handling

Add `catch_unwind` and barrier poisoning to the existing `SpinBarrier`.

**Rejected as insufficient**: fixes the panic deadlock but the O(N) barrier synchronization remains. The barrier forces all workers to synchronize twice per batch, which is the root cause of the 8-core scaling cliff. The architecture must change.

### In-Place Tail Publication (No Swap)

Producer publishes its tail index atomically. Consumer reads directly from the producer's ring.

**Rejected**: requires the ring to support concurrent read + write (SPSC-safe with atomic head/tail). The current `SpillRing` uses `Cell`, not atomics. Would require fundamental changes to the ring's interior mutability model or restrict reads to a safe window. The swap approach keeps `SpillRing` unchanged.

---

## Migration Path

The public API remains backward-compatible:

- `WorkerPool::run(&mut self, args: &A)` still works — calls `dispatch()` then `join()`.
- `Consumer::drain()` still works — collects from handoff slots, then drains the rings.
- `PoolBuilder` API unchanged.

New APIs enable streaming:

- `dispatch(&mut self, args: &A)` — non-blocking dispatch.
- `join(&mut self) -> Result<(), WorkerPanic>` — wait for completion with panic detection.
- `collect()` — collect results while workers may still be running.
- `MergerHandle` — MPSM support via slot partitioning.

Internal changes:

- `SpinBarrier` removed entirely.
- `HandoffSlot` + `WorkerSignal` replace barrier synchronization.
- Worker loop restructured around per-worker signal polling.
- Ring ownership transfers via `AtomicPtr` swap instead of barrier-gated shared access.

---

## What This Fixes

| Current Problem | Resolution |
|----------------|------------|
| Barrier deadlock on worker panic (H-1) | Per-worker signals + `JoinHandle::is_finished()` detection |
| O(N) barrier sync per batch | Independent per-worker signals, no shared counter |
| 8-core scaling cliff | No barrier contention; adaptive spin-then-yield frees CPU |
| `.unwrap()` on join in Drop (H-2) | `join()` returns `Result<(), WorkerPanic>`, Drop handles gracefully |
| `args_ptr` null deref risk (H-3) | Same pattern but with defense-in-depth null check |
| Batch-only execution | Streaming `collect()` overlaps production and consumption |
| No MPSM | Natural slot partitioning across multiple mergers |
| No global ordering | Opt-in sequence stamping; batch-level ordering is free |

---

## Emergent Patterns: TPC + Spout I/O Sequencing

The TPC handoff design and the spout trait create a natural dataflow graph abstraction. Spouts are the edges, rings are the vertices, handoff slots are the cross-thread edges. Three edge types, one trait interface:

- **Intra-thread edges**: spout overflow (zero-cost, `Cell`-based)
- **Cross-thread edges**: handoff slots (one atomic per batch)
- **I/O edges**: terminal spouts (`FramedSpout`, file, network)

The fundamental tension this resolves: **computation is parallel and unordered, but I/O is serial and ordered.** The following patterns emerge at that boundary.

### Pattern 1: Sequenced Fan-In Spout

The consumer already drains N handoff slots into an output spout. Package that as a spout itself:

```rust
/// Collects from N handoff slots, emits to downstream in deterministic order.
/// Implements Spout<T> — composable with any downstream stage.
struct FanInSpout<T, const N: usize, S> {
    slots: Vec<*const HandoffSlot<T, N, S>>,
}

impl<T, const N: usize, S> Spout<T> for FanInSpout<T, N, S> {
    type Error = core::convert::Infallible;

    fn send(&mut self, _item: T) -> Result<(), SendError<T>> {
        // FanInSpout is a source, not a sink — send is a no-op or error.
        // The real work happens in flush() / drain().
        unimplemented!()
    }
}
```

Now TPC collection is composable: `WorkerPool → FanInSpout → FramedSpout → TcpStream`. The spout trait becomes the universal stage connector. Any downstream that accepts `Spout<T>` can consume from a TPC pool without knowing it's parallel underneath.

### Pattern 2: Ordered Completion (io_uring Model)

With per-producer batch sequence numbers (free in the TPC design — plain `u64` increment per batch), a reordering spout buffers out-of-order completions and emits in sequence:

```rust
/// Accepts (batch_seq, items) out of order.
/// Emits to inner spout in strict sequence order.
struct SequencedSpout<T, Inner: Spout<T>> {
    next_expected: u64,
    pending: BTreeMap<u64, Vec<T>>,
    inner: Inner,
}

impl<T, Inner: Spout<T>> SequencedSpout<T, Inner> {
    fn submit(&mut self, batch_seq: u64, items: impl Iterator<Item = T>) {
        if batch_seq == self.next_expected {
            // Fast path: in order, emit directly
            for item in items {
                let _ = self.inner.send(item);
            }
            self.next_expected += 1;
            // Drain any buffered batches that are now in order
            while let Some(buffered) = self.pending.remove(&self.next_expected) {
                for item in buffered {
                    let _ = self.inner.send(item);
                }
                self.next_expected += 1;
            }
        } else {
            // Out of order: buffer until predecessors arrive
            self.pending.insert(batch_seq, items.collect());
        }
    }
}
```

This is io_uring's completion queue pattern — parallel submission, ordered completion — generalized through the spout trait for any I/O backend. The reordering buffer is bounded by the maximum out-of-order distance between producers (typically small: the fastest producer is at most one batch ahead of the slowest).

### Pattern 3: Structural Backpressure Propagation

When the output spout is slow (I/O-bound), backpressure propagates structurally through the system without any explicit flow control:

```
Output spout slow
  → consumer drain slows
    → handoff slots accumulate (batch_ptr stays non-null)
      → producer detects uncollected batch on next publish
        → merges old batch back into active ring
          → ring fills to capacity
            → overflow spout fires (eviction / backpressure callback)
```

Each layer has its own backpressure valve:

| Layer | Backpressure Mechanism | Cost |
|-------|----------------------|------|
| SpillRing overflow | Eviction to overflow spout | Zero (already exists) |
| Handoff slot | Uncollected batch detection | One atomic load per publish |
| Consumer | Drain rate limited by output spout | Zero (structural) |
| Output spout | I/O throughput | External |

No explicit flow control protocol. No tokens. No credits. The ring's finite capacity and the handoff slot's single-entry design create natural backpressure. The overflow spout is the pressure relief valve — configurable per ring (drop, count, collect, spill to disk).

### Pattern 4: Multi-Stage Parallel Pipelines

Since `SpillRing` implements `Spout<T>`, and `WorkerPool` wraps `SpillRing`s, stages chain:

```
Stage 1: WorkerPool<Raw, N, FanInSpout>
            │  (parse raw data, N cores)
            ▼
         FanInSpout ──collects──> batches of Parsed
            │
Stage 2: WorkerPool<Parsed, M, FanInSpout>
            │  (transform/enrich, M cores)
            ▼
         FanInSpout ──collects──> batches of Enriched
            │
Stage 3: SequencedSpout<Enriched, FramedSpout<TcpStream>>
            │  (ordered serialization + network I/O)
            ▼
         TcpStream
```

Each stage is a TPC pool with independent core count. Stages connect via `FanInSpout`. The spout trait is the pipeline glue. Each stage independently scales to its core allocation. The compiler can inline the spout chain — zero dynamic dispatch if types are concrete.

### Pattern 5: I/O Coalescing via Batch Handoff

TPC hands off *rings*, not items. The consumer holds entire contiguous batches after handoff. This enables batch-aware I/O:

```rust
impl<T, const N: usize, S> PoolConsumer<T, N, S> {
    /// Batch-drain: hand entire ring slices to the output spout.
    fn drain_batched<Out: Spout<T>>(&mut self, out: &mut Out)
    where
        T: Copy,
    {
        for batch in self.collect_batches() {
            // send_all gets the full batch — the spout can vectorize I/O
            let _ = out.send_all(batch.drain());
        }
    }
}
```

For `FramedSpout` over a network socket, `send_all` can coalesce the entire batch into a single `writev` syscall. For database spouts, the entire batch becomes one bulk insert. The batching is free — it falls out of TPC's ring-swap handoff. No explicit batching logic needed.

### Pattern 6: MPSM as Parallel Merge Tree

MPSM (multiple mergers) composes the above patterns into a tree:

```
Producers:  P0  P1  P2  P3  P4  P5  P6  P7
              \  |  /     \  |  /     \  |
Mergers:  FanIn[0..2]   FanIn[3..5]  FanIn[6..7]
              │              │             │
              ▼              ▼             ▼
          Merger 0       Merger 1      Merger 2
           (core 8)      (core 9)     (core 10)
              \              │            /
               \             │           /
            FanIn[merger_0..merger_2]
                      │
                      ▼
               SequencedSpout
                      │
                      ▼
                  I/O Spout
```

Each merger is itself a consumer with a `FanInSpout` over a subset of handoff slots. Mergers write to their own output, which feeds the next merge tier. The tree scales logarithmically. Each node in the tree is the same type composition — `FanInSpout` + optional `SequencedSpout` + output spout.

### Design Philosophy: Implicit Composition Over Explicit Pipeline Builder

These patterns compose through Rust's type system rather than a runtime pipeline builder:

```rust
// The user builds the graph by composing types:
let pool = WorkerPoolBuilder::new()
    .workers(8)
    .build(|ring, worker_id, args| {
        // produce into ring
    });

// Collection is just spout composition:
let mut output = SequencedSpout::new(
    FramedSpout::new(tcp_stream)
);

pool.run(&args)?;
pool.collect(&mut output);
```

No `Pipeline` struct. No `Stage` trait. No runtime graph. The types compose, the user builds the graph, the compiler inlines and monomorphizes. Zero-cost abstraction — the assembled pipeline compiles to the same code as a hand-written version.

This is consistent with spill-ring's philosophy: the ring buffer is a building block, the spout trait is the connector, TPC handoff is the cross-thread bridge. The user assembles them. The compiler optimizes them.
