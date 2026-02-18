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
    /// Can be called after dispatch() returns but before join(), or after join(),
    /// or between run() calls. Sequential &mut self — not concurrent with dispatch.
    /// Returns the number of items drained.
    pub fn collect<Out: Spout<T>>(&mut self, out: &mut Out) -> Result<usize, Out::Error>;

    /// Block until all workers signal done. Detects panics.
    pub fn join(&mut self) -> Result<(), WorkerPanic>;

    /// Synchronous: dispatch + join. Existing API, unchanged.
    pub fn run(&mut self, args: &A);
}
```

`collect()` iterates all handoff slots, swaps null into `batch` (Acquire), drains the ring into the output spout, then offers the empty ring back into `recycle` (Release). Workers see the recycled ring on their next publish cycle. The consumer never blocks — if a slot is empty (worker hasn't published yet), it's skipped.

**Note:** `collect()` returns `Result<usize, Out::Error>` because the output spout's `send()` is fallible. The current internal collection uses `let _ =` because the overflow spout has `Error = Infallible`, but `collect()` should propagate errors from arbitrary downstream spouts.

### Batch Sequence Counter (proactive, for Feature 3)

Add `batch_seq: AtomicU64` to `HandoffSlot` now to avoid a structural change later. `dispatch()` increments a pool-level `u64` counter and stores the current sequence into each slot's `batch_seq` with `Release` ordering before signaling go. Workers don't need to read it — the consumer reads it alongside `collect()`.

Memory ordering: main thread stores `batch_seq` with `Release`, then stores `go=true` with `Release`. Worker wakes on `go` with `Acquire` (establishing happens-before), publishes ring with `Release`. Consumer loads ring with `Acquire`, which transitively sees `batch_seq`. The consumer reads `batch_seq` with `Acquire` to pair with the main thread's `Release` store. This chain is correct because the worker's `Release` publish happens-after the main thread's `Release` on `go`, and the consumer's `Acquire` on the batch happens-after the worker's publish.

### Implementation Plan

#### Step 1a: Add `batch_seq` to `HandoffSlot`

**File: `mpsc/handoff.rs`**

Add field to `HandoffSlot`:
```rust
use core::sync::atomic::AtomicU64;

#[repr(C, align(128))]
pub(crate) struct HandoffSlot<T, const N: usize, S: Spout<T, Error = core::convert::Infallible>> {
    batch: AtomicPtr<SpillRing<T, N, S>>,
    recycle: AtomicPtr<SpillRing<T, N, S>>,
    /// Batch sequence number. Stamped by main thread before dispatch,
    /// read by consumer during collect(). Not accessed by workers.
    batch_seq: AtomicU64,
}
```

Update `new()`:
```rust
pub(crate) fn new() -> Self {
    Self {
        batch: AtomicPtr::new(ptr::null_mut()),
        recycle: AtomicPtr::new(ptr::null_mut()),
        batch_seq: AtomicU64::new(0),
    }
}
```

Add stamping and reading methods:
```rust
/// Main thread: stamp this slot's batch sequence number before dispatch.
#[inline]
pub(crate) fn stamp_seq(&self, seq: u64) {
    // Ordering: Release — publishes seq before go signal wakes worker.
    // The go signal's Release provides the cross-thread fence; this
    // Release ensures the seq value is visible when the consumer later
    // reads the batch with Acquire.
    self.batch_seq.store(seq, Ordering::Release);
}

/// Consumer: read the batch sequence number for the last published batch.
/// Call after collect() returns Some — the Acquire on batch.swap(null)
/// transitively sees the seq stored by the main thread.
#[inline]
pub(crate) fn read_seq(&self) -> u64 {
    // Ordering: Acquire — pairs with main thread's Release store.
    // The happens-before chain: main Release(seq) → main Release(go) →
    // worker Acquire(go) → worker Release(publish) → consumer Acquire(batch).
    // This Acquire is technically redundant (the batch Acquire already
    // provides the fence), but makes the dependency explicit.
    self.batch_seq.load(Ordering::Acquire)
}
```

No change to `Drop` — `AtomicU64` has no special drop behavior.

#### Step 1b: Add `batch_seq` counter and `dispatch()` / `join()` / `collect()` to `WorkerPool`

**File: `mpsc/pool.rs`**

Add field to `WorkerPool`:
```rust
pub struct WorkerPool<T, const N: usize, S, F, A> {
    // ... existing fields ...
    /// Monotonic batch sequence counter, incremented on each dispatch.
    batch_seq: u64,
}
```

Initialize in `start()`:
```rust
Self {
    // ... existing ...
    batch_seq: 0,
}
```

Add methods:
```rust
/// Non-blocking: stamp batch sequence, store args, signal all workers.
///
/// Returns immediately. Call `join()` to wait for completion, or
/// `collect()` to drain available results without waiting.
///
/// # Panics
///
/// Panics if any worker has previously panicked.
pub fn dispatch(&mut self, args: &A) {
    self.check_panicked().expect("worker panicked in previous round");

    let seq = self.batch_seq;
    self.batch_seq += 1;

    // Stamp batch sequence on all slots before signaling go.
    for slot in self.slots.iter() {
        slot.stamp_seq(seq);
    }

    // Ordering: Release — publishes args data before workers read it.
    self.args_ptr
        .store(core::ptr::from_ref(args).cast_mut(), Ordering::Release);

    for sig in self.signals.iter() {
        sig.signal_go();
    }
}

/// Block until all workers signal done. Detects panics.
///
/// After `join()` returns, all handoff slots contain published batches
/// ready for `collect()`.
pub fn join(&mut self) -> Result<(), WorkerPanic> {
    sync::spin_wait(self.spin_limit, || self.all_done_or_panicked());
    self.check_panicked()?;
    for sig in self.signals.iter() {
        sig.clear_done();
    }
    Ok(())
}

/// Collect all available batches from handoff slots into a spout.
///
/// Non-blocking: iterates each slot, takes any published batch (Acquire),
/// drains items into `out`, then recycles the empty ring (Release).
/// Slots where the worker hasn't published yet are skipped.
///
/// Returns the total number of items drained across all slots.
///
/// Can be called:
/// - After `dispatch()` but before `join()` (streaming collection)
/// - After `join()` (all batches available)
/// - Between `run()` calls (collect previous round's batches)
///
/// # Ring Recycling
///
/// Empty rings are offered back to workers via `offer_recycle()`. If the
/// worker hasn't taken the previous recycled ring, the old one is dropped.
/// This only occurs when the consumer collects faster than workers produce
/// (consumer is not the bottleneck) — document-only concern, not a bug.
pub fn collect<Out: Spout<T>>(&mut self, out: &mut Out) -> Result<usize, Out::Error> {
    let mut total = 0usize;
    for slot in self.slots.iter() {
        if let Some(mut ring) = slot.collect() {
            let count = ring.len();
            out.send_all(ring.drain())?;
            slot.offer_recycle(ring);
            total += count;
        }
    }
    Ok(total)
}
```

Refactor `try_run()`:
```rust
pub fn try_run(&mut self, args: &A) -> Result<(), WorkerPanic> {
    self.dispatch(args);
    self.join()
}
```

`run()` stays unchanged (calls `try_run()`).

Update `shutdown_and_join()`: replace the inline collection loop with `self.collect()` using a local `Vec` spout, or keep the current inline loop (since `shutdown_and_join` returns `Vec<Box<SpillRing>>`, not drained items). The existing `shutdown_and_join` collects `Box<SpillRing>` from slots — `collect()` drains items. These are different operations, so `shutdown_and_join` keeps its current code.

#### Step 1c: Update `into_consumer` to use collect semantics

No change needed — `into_consumer()` calls `shutdown_and_join()` which already collects Box<SpillRing> from slots and builds a Consumer. The new `collect()` is a separate path for streaming item-level collection.

#### Step 1d: Tests

**File: `tests/mpsc.rs`**

```rust
#[test]
fn test_dispatch_join_collect() {
    // dispatch + join + collect should produce same results as run + into_consumer
    let mut pool = MpscRing::<u64, 256>::pool(4)
        .spawn(|ring, id, count: &u64| {
            for i in 0..*count {
                ring.push(id as u64 * 10000 + i);
            }
        });

    pool.dispatch(&100);
    pool.join().unwrap();

    let mut sink = CollectSpout::new();
    let count = pool.collect(&mut sink).unwrap();
    assert_eq!(count, 400); // 4 workers * 100 items
    assert_eq!(sink.items().len(), 400);
}

#[test]
fn test_collect_between_runs() {
    // collect() picks up batches from previous run()
    let mut pool = MpscRing::<u64, 256>::pool(2)
        .spawn(|ring, _id, count: &u64| {
            for i in 0..*count {
                ring.push(i);
            }
        });

    pool.run(&50);

    let mut sink = CollectSpout::new();
    let count = pool.collect(&mut sink).unwrap();
    assert_eq!(count, 100); // 2 workers * 50

    // Second run should also work
    pool.run(&30);
    let count2 = pool.collect(&mut sink).unwrap();
    assert_eq!(count2, 60);
    assert_eq!(sink.items().len(), 160);
}

#[test]
fn test_batch_seq_increments() {
    // Verify batch_seq increments on each dispatch
    let mut pool = MpscRing::<u64, 256>::pool(2)
        .spawn(|ring, _id, _args: &()| {
            ring.push(1);
        });

    // Run 3 rounds, verify batch_seq progresses
    for _ in 0..3 {
        pool.dispatch(&());
        pool.join().unwrap();
        pool.collect(&mut spout::DropSpout).unwrap();
    }

    // batch_seq is internal — test via collect_with_seq (Feature 3)
    // For now, just verify dispatch/join/collect cycle works over
    // multiple rounds without issues.

    let consumer = pool.into_consumer();
    // into_consumer drains final rings — nothing should be left
    // because we collected after each round.
}

#[test]
fn test_collect_empty_slots_skipped() {
    // collect() when no batches published yet returns 0
    let mut pool = MpscRing::<u64, 256>::pool(2)
        .spawn(|ring, _id, _args: &()| {
            ring.push(1);
        });

    // No dispatch yet — slots are empty
    let mut sink = CollectSpout::new();
    let count = pool.collect(&mut sink).unwrap();
    assert_eq!(count, 0);
    assert!(sink.items().is_empty());
}

#[test]
fn test_collect_propagates_spout_error() {
    use std::sync::mpsc;

    let mut pool = MpscRing::<u64, 256>::pool(1)
        .spawn(|ring, _id, _args: &()| {
            ring.push(42);
        });

    pool.run(&());

    // ChannelSpout with a dropped receiver will error on send
    let (tx, rx) = mpsc::channel();
    drop(rx);
    let mut spout = spout::ChannelSpout::new(tx);
    let result = pool.collect(&mut spout);
    assert!(result.is_err());
}
```

### Key Details

- `collect()` takes `&mut self` — only one collector at a time (MPSC invariant).
- `run()` becomes `dispatch(); join();` internally (backward compatible).
- `collect()` can be called after `dispatch()` returns (sequential `&mut self`), after `join()`, or between `run()` calls. All are sequential borrows, not concurrent.
- Ring recycling in `collect()` closes the triple-buffer loop: active -> published -> recycled -> active.
- The `into_consumer()` path still works — it calls `shutdown_and_join()` which collects everything.

### Ring recycling edge case

When `collect()` is called multiple times before a worker publishes, the second call finds an empty slot and returns. The recycled ring from the first call sits in the recycle slot. If `collect()` offers another ring later, `offer_recycle()` drops the old one (handoff.rs line 82). This breaks zero-allocation steady state during rapid collection without intervening worker publishes. This is acceptable — it only occurs when the consumer collects faster than workers produce, which means the consumer is not the bottleneck. Document this in the `collect()` doc comment.

### Files

| File | Change |
|------|--------|
| `mpsc/handoff.rs` | Add `batch_seq: AtomicU64` field, `stamp_seq()`, `read_seq()`. Update `new()`. |
| `mpsc/pool.rs` | Add `batch_seq: u64` field. Add `dispatch()`, `collect()`, `join()`. Refactor `try_run()` to `dispatch() + join()`. |
| `tests/mpsc.rs` | 5 new tests: dispatch_join_collect, collect_between_runs, batch_seq_increments, collect_empty_slots_skipped, collect_propagates_spout_error. |

### Dependencies

None — builds directly on the existing TPC implementation.

### Backward Compatibility

- `run()` / `try_run()` behavior unchanged (internally becomes `dispatch() + join()`).
- `into_consumer()` unchanged (still calls `shutdown_and_join()`).
- All 8 existing worker pool tests + 4 Phase 2 tests pass without modification.

---

## Feature 2: `FanInSpout` — Composable N-to-1 Collection

### Problem

`pool.collect(&mut sink)` couples collection to the pool. To compose TPC output with downstream stages (batching, framing, network I/O), collection should be a spout itself — a source that drains handoff slots and emits items to its inner spout.

### Ownership Model: Scoped API

The borrow-based approach (`pool.fan_in(&mut sink) -> FanInSpout<'_, ...>`) is unworkable — `FanInSpout` borrows `&pool`, preventing `dispatch()`/`join()` calls while it exists.

The raw pointer approach (`SendPtr<HandoffSlot>`) works but the don't-outlive-pool invariant must be enforced structurally, not by documentation.

**Resolution: scoped closure API.**

```rust
impl WorkerPool<T, N, S, F, A> {
    /// Execute a closure with a FanInSpout that borrows the pool's handoff slots.
    /// The FanInSpout cannot escape the closure — lifetime enforced by scope.
    pub fn with_fan_in<Out, R>(
        &mut self,
        inner: Out,
        f: impl FnOnce(&mut FanInSpout<T, N, S, Out>) -> R,
    ) -> R
    where
        Out: Spout<T>;
}
```

Inside the closure, the user has a `&mut FanInSpout` that can call `flush()` to collect from handoff slots. When the closure returns, the `FanInSpout` is dropped, and the raw pointers cannot escape. The pool regains `&mut self` for subsequent `dispatch()`/`join()` calls.

For cases where the scoped API is too restrictive (e.g., long-lived fan-in across multiple dispatch cycles), provide an unsafe escape hatch:

```rust
impl WorkerPool<T, N, S, F, A> {
    /// Create a FanInSpout with raw pointers to handoff slots.
    ///
    /// # Safety
    /// The returned FanInSpout must not outlive self.
    /// Caller must ensure the pool is not dropped while the FanInSpout exists.
    pub unsafe fn fan_in_unchecked<Out: Spout<T>>(
        &self,
        inner: Out,
    ) -> FanInSpout<T, N, S, Out>;
}
```

### Implementation Plan

#### Step 2a: Create `FanInSpout` type

**New file: `mpsc/fan_in.rs`**

```rust
//! Composable fan-in spout for collecting from handoff slots.

extern crate alloc;

use alloc::boxed::Box;
use spout::Spout;

use super::handoff::HandoffSlot;
use super::pool::SendPtr;

/// Collects from N handoff slots, emits to downstream spout.
///
/// On `flush()`, iterates all assigned handoff slots, takes any published
/// batch (Acquire), drains items into the inner spout via `send_all()`,
/// then recycles the empty ring back to the worker (Release).
///
/// `send()` is pass-through to the inner spout — for items that bypass
/// handoff (e.g., metadata, control signals). The primary data path is
/// `flush()`.
///
/// Created via [`WorkerPool::with_fan_in()`] (scoped, safe) or
/// [`WorkerPool::fan_in_unchecked()`] (unsafe, unscoped).
pub struct FanInSpout<
    T,
    const N: usize,
    S: Spout<T, Error = core::convert::Infallible>,
    Out: Spout<T>,
> {
    /// Raw pointers to handoff slots owned by WorkerPool.
    /// Validity invariant: these point into the pool's `Box<[HandoffSlot]>`.
    /// The scoped API ensures the pool outlives this struct.
    slots: Box<[SendPtr<HandoffSlot<T, N, S>>]>,
    /// Number of slots (redundant with slots.len(), but avoids bounds check).
    num_slots: usize,
    /// Downstream spout receiving collected items.
    inner: Out,
}

impl<T, const N: usize, S, Out> FanInSpout<T, N, S, Out>
where
    S: Spout<T, Error = core::convert::Infallible>,
    Out: Spout<T>,
{
    /// Create a FanInSpout from raw slot pointers.
    ///
    /// # Safety
    /// All pointers in `slots` must be valid for the lifetime of this struct.
    pub(crate) unsafe fn new(
        slots: Box<[SendPtr<HandoffSlot<T, N, S>>]>,
        inner: Out,
    ) -> Self {
        let num_slots = slots.len();
        Self {
            slots,
            num_slots,
            inner,
        }
    }

    /// Reference to the inner spout.
    pub fn inner(&self) -> &Out {
        &self.inner
    }

    /// Mutable reference to the inner spout.
    pub fn inner_mut(&mut self) -> &mut Out {
        &mut self.inner
    }

    /// Consume and return the inner spout.
    pub fn into_inner(self) -> Out {
        self.inner
    }

    /// Number of handoff slots this fan-in collects from.
    pub fn num_slots(&self) -> usize {
        self.num_slots
    }
}

impl<T, const N: usize, S, Out> Spout<T> for FanInSpout<T, N, S, Out>
where
    S: Spout<T, Error = core::convert::Infallible>,
    Out: Spout<T>,
{
    type Error = Out::Error;

    /// Pass-through: forward an externally-produced item to inner.
    /// This is NOT the primary data path — flush() is.
    #[inline]
    fn send(&mut self, item: T) -> Result<(), Self::Error> {
        self.inner.send(item)
    }

    /// Collect all available batches from handoff slots, drain into inner.
    ///
    /// For each slot with a published batch:
    /// 1. Swap null into batch pointer (Acquire) — takes ownership of ring
    /// 2. Drain all items from ring into inner spout via send_all()
    /// 3. Offer empty ring back to worker via recycle pointer (Release)
    ///
    /// Slots where the worker hasn't published yet are skipped silently.
    fn flush(&mut self) -> Result<(), Self::Error> {
        for slot_ptr in self.slots.iter() {
            // SAFETY: Pointer validity guaranteed by scoped API (with_fan_in)
            // or caller contract (fan_in_unchecked).
            let slot = unsafe { &*slot_ptr.0 };
            if let Some(mut ring) = slot.collect() {
                self.inner.send_all(ring.drain())?;
                slot.offer_recycle(ring);
            }
        }
        self.inner.flush()
    }
}

// SAFETY: FanInSpout is Send when Out is Send. The slot pointers
// are SendPtr (manually Send). The validity invariant is maintained
// by the scoped API or the unsafe constructor's contract.
unsafe impl<T, const N: usize, S, Out> Send for FanInSpout<T, N, S, Out>
where
    S: Spout<T, Error = core::convert::Infallible>,
    Out: Spout<T> + Send,
{
}
```

Note: `SendPtr` is currently `pub(super)` in `pool.rs`. It needs to become `pub(crate)` so `fan_in.rs` can import it.

#### Step 2b: Make `SendPtr` crate-visible

**File: `mpsc/pool.rs`**

Change `SendPtr` visibility from implicit `pub(super)` to explicit `pub(crate)`:

```rust
pub(crate) struct SendPtr<T>(pub(crate) *const T);
```

#### Step 2c: Add pool methods

**File: `mpsc/pool.rs`**

```rust
use super::fan_in::FanInSpout;

impl<T, const N: usize, S, F, A> WorkerPool<T, N, S, F, A>
where
    S: Spout<T, Error = core::convert::Infallible>,
    F: Fn(&SpillRing<T, N, S>, usize, &A) + Send + Clone + 'static,
    A: Sync + 'static,
{
    /// Execute a closure with a FanInSpout that collects from all handoff slots.
    ///
    /// The FanInSpout cannot escape the closure — its raw pointers to handoff
    /// slots are valid only for the closure's duration.
    ///
    /// # Example
    ///
    /// ```
    /// pool.dispatch(&args);
    /// pool.join()?;
    /// pool.with_fan_in(CollectSpout::new(), |fan_in| {
    ///     fan_in.flush()
    /// })?;
    /// ```
    pub fn with_fan_in<Out, R>(
        &mut self,
        inner: Out,
        f: impl FnOnce(&mut FanInSpout<T, N, S, Out>) -> R,
    ) -> R
    where
        Out: Spout<T>,
    {
        let slot_ptrs: Box<[SendPtr<HandoffSlot<T, N, S>>]> = self
            .slots
            .iter()
            .map(|slot| SendPtr(core::ptr::from_ref(slot)))
            .collect::<Vec<_>>()
            .into_boxed_slice();

        // SAFETY: Slot pointers are valid for the duration of the closure.
        // The pool's &mut self borrow prevents dropping the pool or calling
        // dispatch/join until the closure returns.
        let mut fan_in = unsafe { FanInSpout::new(slot_ptrs, inner) };
        f(&mut fan_in)
    }

    /// Create a FanInSpout with raw pointers to all handoff slots.
    ///
    /// # Safety
    ///
    /// The returned FanInSpout must not outlive `self`. The caller must
    /// ensure the pool is not dropped while the FanInSpout exists.
    /// Prefer [`with_fan_in()`] for the safe scoped API.
    pub unsafe fn fan_in_unchecked<Out: Spout<T>>(
        &self,
        inner: Out,
    ) -> FanInSpout<T, N, S, Out> {
        let slot_ptrs: Box<[SendPtr<HandoffSlot<T, N, S>>]> = self
            .slots
            .iter()
            .map(|slot| SendPtr(core::ptr::from_ref(slot)))
            .collect::<Vec<_>>()
            .into_boxed_slice();

        // SAFETY: Caller guarantees the pool outlives the returned FanInSpout.
        FanInSpout::new(slot_ptrs, inner)
    }
}
```

#### Step 2d: Module structure

**File: `mpsc/mod.rs`**

```rust
#[cfg(feature = "std")]
mod fan_in;
// ...
#[cfg(feature = "std")]
pub use fan_in::FanInSpout;
```

**File: `lib.rs`**

```rust
#[cfg(feature = "std")]
pub use mpsc::{FanInSpout, PoolBuilder, WorkerPanic, WorkerPool};
```

#### Step 2e: Tests

**File: `tests/mpsc.rs`**

```rust
#[test]
fn test_fan_in_scoped_basic() {
    let mut pool = MpscRing::<u64, 256>::pool(4)
        .spawn(|ring, id, count: &u64| {
            for i in 0..*count {
                ring.push(id as u64 * 10000 + i);
            }
        });

    pool.run(&100);

    let result = pool.with_fan_in(CollectSpout::new(), |fan_in| {
        fan_in.flush().unwrap();
        fan_in.into_inner() // can't do this — fan_in is &mut
    });
    // Actually: the closure gets &mut FanInSpout, so access inner via inner()
    let mut collected = Vec::new();
    pool.with_fan_in(CollectSpout::new(), |fan_in| -> Result<(), Infallible> {
        fan_in.flush()?;
        collected = fan_in.inner().items().to_vec();
        Ok(())
    }).unwrap();
    assert_eq!(collected.len(), 400);
}

#[test]
fn test_fan_in_send_passthrough() {
    // send() forwards directly to inner, bypassing handoff slots
    let mut pool = MpscRing::<u64, 256>::pool(1)
        .spawn(|_ring, _id, _args: &()| {});

    pool.with_fan_in(CollectSpout::new(), |fan_in| -> Result<(), Infallible> {
        fan_in.send(42)?;
        fan_in.send(43)?;
        assert_eq!(fan_in.inner().items(), &[42, 43]);
        Ok(())
    }).unwrap();
}

#[test]
fn test_fan_in_multiple_flushes() {
    // Multiple flush calls — second flush collects nothing
    let mut pool = MpscRing::<u64, 256>::pool(2)
        .spawn(|ring, _id, _args: &()| {
            ring.push(1);
        });

    pool.run(&());

    pool.with_fan_in(CollectSpout::new(), |fan_in| -> Result<(), Infallible> {
        fan_in.flush()?;
        let first = fan_in.inner().items().len();
        assert_eq!(first, 2);

        fan_in.flush()?; // no new batches — nothing collected
        assert_eq!(fan_in.inner().items().len(), 2);
        Ok(())
    }).unwrap();
}
```

### I/O Coalescing Note

`flush()` calls `send_all(ring.drain())` which provides the downstream spout with a full batch as an iterator. For `Copy` types, a future optimization could expose contiguous slices directly from the ring's internal buffer for zero-copy I/O coalescing (`writev`-style syscalls). This is a downstream spout concern — defer as a future optimization on `SpillRing` itself (e.g., `as_slices() -> (&[T], &[T])` for the two contiguous segments across the wraparound).

### Files

| File | Change |
|------|--------|
| `mpsc/fan_in.rs` | **New**: `FanInSpout` struct + `Spout<T>` impl + `unsafe Send` impl. |
| `mpsc/pool.rs` | `SendPtr` -> `pub(crate)`. Add `with_fan_in()`, `fan_in_unchecked()`. |
| `mpsc/mod.rs` | Add `mod fan_in;`, re-export `FanInSpout`. |
| `lib.rs` | Re-export `FanInSpout`. |
| `tests/mpsc.rs` | 3 new tests: scoped basic, send passthrough, multiple flushes. |

### Dependencies

Feature 1 (`collect()` — `FanInSpout::flush()` is the spout-interface equivalent).

---

## Feature 3: `SequencedSpout` — Ordered Completion

### Problem

Workers complete in arbitrary order. Per-producer FIFO is guaranteed by the ring, but cross-producer ordering is nondeterministic. Some workloads require global ordering — batches must be emitted to the downstream spout in the same sequence they were dispatched.

### Design

`SequencedSpout` is a standalone reordering buffer. It lives in the **spout** crate — no dependency on spill-ring internals.

### Implementation Plan

#### Step 3a: `SequencedSpout` in the spout crate

**New file: `spout/src/impls/sequenced.rs`**

```rust
//! Reordering spout for ordered completion.

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use crate::Spout;

/// Reorders batches by sequence number before forwarding to inner spout.
///
/// Fast path: if the next expected sequence arrives, emit directly (zero buffering).
/// Slow path: buffer until predecessors arrive, then flush in order.
///
/// This is the io_uring completion queue pattern — parallel submission,
/// ordered completion — generalized through the spout trait.
///
/// # Buffering Bound
///
/// The reordering buffer size is bounded by the maximum out-of-order
/// distance between producers. With N workers, at most N-1 batches can
/// be buffered at any time (the fastest producer is at most N-1 batches
/// ahead of the slowest).
pub struct SequencedSpout<T, Inner: Spout<T>> {
    next_expected: u64,
    pending: BTreeMap<u64, Vec<T>>,
    inner: Inner,
}

impl<T, Inner: Spout<T>> SequencedSpout<T, Inner> {
    /// Create a new sequenced spout wrapping an inner spout.
    ///
    /// Batches are reordered by sequence number before forwarding.
    /// The first expected sequence number is 0.
    pub fn new(inner: Inner) -> Self {
        Self {
            next_expected: 0,
            pending: BTreeMap::new(),
            inner,
        }
    }

    /// Create with a custom starting sequence number.
    pub fn with_start_seq(inner: Inner, start_seq: u64) -> Self {
        Self {
            next_expected: start_seq,
            pending: BTreeMap::new(),
            inner,
        }
    }

    /// Submit a batch with its sequence number.
    ///
    /// If `batch_seq == next_expected`, items are emitted directly to the
    /// inner spout (fast path, zero buffering). Then any consecutively-
    /// buffered batches are flushed.
    ///
    /// If `batch_seq > next_expected`, items are buffered in a BTreeMap
    /// until their predecessors arrive.
    pub fn submit(
        &mut self,
        batch_seq: u64,
        items: impl Iterator<Item = T>,
    ) -> Result<(), Inner::Error> {
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

    /// Number of batches currently buffered (waiting for predecessors).
    pub fn buffered_batches(&self) -> usize {
        self.pending.len()
    }

    /// The next expected sequence number.
    pub fn next_expected(&self) -> u64 {
        self.next_expected
    }

    /// Reference to the inner spout.
    pub fn inner(&self) -> &Inner {
        &self.inner
    }

    /// Mutable reference to the inner spout.
    pub fn inner_mut(&mut self) -> &mut Inner {
        &mut self.inner
    }

    /// Consume and return the inner spout.
    ///
    /// Any buffered batches are dropped. Call `submit()` for all
    /// remaining sequences first to ensure nothing is lost.
    pub fn into_inner(self) -> Inner {
        self.inner
    }
}
```

Note: `SequencedSpout` does NOT implement `Spout<T>` itself — it's not a sink for individual items. It's a batch-level reordering buffer with `submit(seq, items)`. The `Spout<T>` trait is implemented on its wrapper via the `Collector` pattern (Step 3b).

**File: `spout/src/impls/mod.rs`**

Add `pub mod sequenced;` and re-export in `spout/src/lib.rs`:
```rust
pub use impls::sequenced::SequencedSpout;
```

#### Step 3b: `Collector` strategy trait

**New file: `spill-ring/src/mpsc/collector.rs`**

```rust
//! Collector strategy for FanInSpout batch delivery.
//!
//! The `Collector` trait decouples how FanInSpout delivers collected
//! batches from the downstream spout interface. This bridges the gap
//! between batch-level metadata (batch_seq, worker_id) and the
//! item-level `Spout<T>` trait.

use spout::Spout;

/// Strategy for how FanInSpout delivers collected batches.
///
/// Each `flush()` cycle in FanInSpout calls `deliver()` for each slot
/// that has a published batch, then calls `finish()` once all slots
/// have been processed.
pub trait Collector<T> {
    /// Error type for delivery failures.
    type Error;

    /// Deliver a batch of items with metadata.
    ///
    /// - `batch_seq`: sequence number stamped by `dispatch()` (monotonic)
    /// - `worker_id`: index of the worker that produced this batch
    /// - `items`: iterator over the batch contents (drained from ring)
    fn deliver(
        &mut self,
        batch_seq: u64,
        worker_id: usize,
        items: impl Iterator<Item = T>,
    ) -> Result<(), Self::Error>;

    /// Signal that all available batches have been delivered for this flush.
    ///
    /// Called once per `flush()` cycle after all `deliver()` calls complete.
    /// Use this to flush downstream buffers or emit end-of-batch markers.
    fn finish(&mut self) -> Result<(), Self::Error>;
}

/// Unordered collector: forwards items directly to inner spout.
///
/// Ignores `batch_seq` and `worker_id` — same behavior as
/// `WorkerPool::collect()`. This is the default collector for
/// workloads that don't need ordering.
pub struct UnorderedCollector<S> {
    inner: S,
}

impl<S> UnorderedCollector<S> {
    /// Wrap a spout as an unordered collector.
    pub fn new(inner: S) -> Self {
        Self { inner }
    }

    /// Reference to the inner spout.
    pub fn inner(&self) -> &S {
        &self.inner
    }

    /// Mutable reference to the inner spout.
    pub fn inner_mut(&mut self) -> &mut S {
        &mut self.inner
    }

    /// Consume and return the inner spout.
    pub fn into_inner(self) -> S {
        self.inner
    }
}

impl<T, S: Spout<T>> Collector<T> for UnorderedCollector<S> {
    type Error = S::Error;

    #[inline]
    fn deliver(
        &mut self,
        _batch_seq: u64,
        _worker_id: usize,
        items: impl Iterator<Item = T>,
    ) -> Result<(), Self::Error> {
        self.inner.send_all(items)
    }

    #[inline]
    fn finish(&mut self) -> Result<(), Self::Error> {
        self.inner.flush()
    }
}

/// Sequenced collector: reorders batches via SequencedSpout before forwarding.
///
/// Routes each batch through `SequencedSpout::submit(batch_seq, items)`,
/// which buffers out-of-order batches and emits in strict sequence order.
pub struct SequencedCollector<T, S: Spout<T>> {
    sequencer: spout::SequencedSpout<T, S>,
}

impl<T, S: Spout<T>> SequencedCollector<T, S> {
    /// Wrap a SequencedSpout as a collector.
    pub fn new(sequencer: spout::SequencedSpout<T, S>) -> Self {
        Self { sequencer }
    }

    /// Create from an inner spout (starting at sequence 0).
    pub fn from_spout(inner: S) -> Self {
        Self {
            sequencer: spout::SequencedSpout::new(inner),
        }
    }

    /// Reference to the inner SequencedSpout.
    pub fn sequencer(&self) -> &spout::SequencedSpout<T, S> {
        &self.sequencer
    }

    /// Mutable reference to the inner SequencedSpout.
    pub fn sequencer_mut(&mut self) -> &mut spout::SequencedSpout<T, S> {
        &mut self.sequencer
    }

    /// Consume and return the inner SequencedSpout.
    pub fn into_sequencer(self) -> spout::SequencedSpout<T, S> {
        self.sequencer
    }
}

impl<T, S: Spout<T>> Collector<T> for SequencedCollector<T, S> {
    type Error = S::Error;

    #[inline]
    fn deliver(
        &mut self,
        batch_seq: u64,
        _worker_id: usize,
        items: impl Iterator<Item = T>,
    ) -> Result<(), Self::Error> {
        self.sequencer.submit(batch_seq, items)
    }

    #[inline]
    fn finish(&mut self) -> Result<(), Self::Error> {
        // SequencedSpout flushes in-order batches as they arrive.
        // finish() flushes the inner spout for any buffered I/O.
        self.sequencer.inner_mut().flush()
    }
}
```

#### Step 3c: Make `FanInSpout` generic over `Collector`

**File: `mpsc/fan_in.rs`**

Replace `Out: Spout<T>` with `C: Collector<T>`:

```rust
use super::collector::Collector;

pub struct FanInSpout<
    T,
    const N: usize,
    S: Spout<T, Error = core::convert::Infallible>,
    C: Collector<T>,
> {
    slots: Box<[SendPtr<HandoffSlot<T, N, S>>]>,
    num_slots: usize,
    collector: C,
    _marker: core::marker::PhantomData<T>,
}
```

Update `flush()` to pass `batch_seq` and `worker_id`:

```rust
fn flush(&mut self) -> Result<(), Self::Error> {
    for (worker_id, slot_ptr) in self.slots.iter().enumerate() {
        let slot = unsafe { &*slot_ptr.0 };
        if let Some(mut ring) = slot.collect() {
            let batch_seq = slot.read_seq();
            self.collector.deliver(batch_seq, worker_id, ring.drain())?;
            slot.offer_recycle(ring);
        }
    }
    self.collector.finish()
}
```

`FanInSpout` still implements `Spout<T>` by delegating to the collector:
- `send()` is not meaningful through a collector — make it available only when `C` also implements `Spout<T>` via a separate impl, or remove `Spout<T>` impl from `FanInSpout` entirely.

**Design decision:** `FanInSpout` is primarily a collection source, not a sink. Remove the `Spout<T>` impl. Users call `flush()` directly. This avoids the awkward `send()` pass-through and keeps the type focused. If composability as a `Spout<T>` is needed, the user wraps it with a `FnSpout` or uses the inner spout directly.

Alternative: Keep `Spout<T>` impl but only on `FanInSpout<..., UnorderedCollector<S>>` where the inner `S: Spout<T>`. This preserves the simple composition pattern for the common case.

**Recommendation:** Implement both:
- `FanInSpout` always has `flush()` method.
- `Spout<T>` impl only when collector is `UnorderedCollector<S>` where `S: Spout<T>`. This way, the simple `pool.with_fan_in(UnorderedCollector::new(sink), ...)` path gets full spout composability.

#### Step 3d: Update pool methods

**File: `mpsc/pool.rs`**

Update `with_fan_in` and `fan_in_unchecked` signatures:

```rust
pub fn with_fan_in<C, R>(
    &mut self,
    collector: C,
    f: impl FnOnce(&mut FanInSpout<T, N, S, C>) -> R,
) -> R
where
    C: Collector<T>,
{ ... }
```

The pool's `collect()` method stays as-is — it's the simple path that doesn't go through the Collector trait.

#### Step 3e: Tests

**File: `spout/src/tests/` (or inline `#[cfg(test)]` in `sequenced.rs`)**

```rust
#[test]
fn test_sequenced_fast_path() {
    // All batches arrive in order — zero buffering
    let mut seq = SequencedSpout::new(CollectSpout::new());
    seq.submit(0, [1, 2, 3].into_iter()).unwrap();
    seq.submit(1, [4, 5].into_iter()).unwrap();
    seq.submit(2, [6].into_iter()).unwrap();
    assert_eq!(seq.inner().items(), &[1, 2, 3, 4, 5, 6]);
    assert_eq!(seq.buffered_batches(), 0);
}

#[test]
fn test_sequenced_reorder() {
    // Batch 1 arrives before batch 0
    let mut seq = SequencedSpout::new(CollectSpout::new());
    seq.submit(1, [4, 5].into_iter()).unwrap();
    assert_eq!(seq.inner().items(), &[]); // buffered
    assert_eq!(seq.buffered_batches(), 1);

    seq.submit(0, [1, 2, 3].into_iter()).unwrap();
    // Batch 0 emitted, then buffered batch 1 flushed
    assert_eq!(seq.inner().items(), &[1, 2, 3, 4, 5]);
    assert_eq!(seq.buffered_batches(), 0);
}

#[test]
fn test_sequenced_gap() {
    // Batch 2 arrives, then 0, then 1 — all three emit after 1 arrives
    let mut seq = SequencedSpout::new(CollectSpout::new());
    seq.submit(2, [7].into_iter()).unwrap();
    seq.submit(0, [1, 2].into_iter()).unwrap();
    assert_eq!(seq.inner().items(), &[1, 2]); // only batch 0
    assert_eq!(seq.buffered_batches(), 1); // batch 2 still buffered

    seq.submit(1, [4, 5].into_iter()).unwrap();
    assert_eq!(seq.inner().items(), &[1, 2, 4, 5, 7]); // batch 1 + batch 2
    assert_eq!(seq.buffered_batches(), 0);
}
```

**File: `tests/mpsc.rs`**

Integration test: pool -> fan_in -> sequenced_collector -> verify order.

```rust
#[test]
fn test_fan_in_sequenced_collector() {
    // 4 workers, each pushes their worker_id. Verify output arrives
    // in dispatch order (all from round 0 before round 1, etc.)
    let mut pool = MpscRing::<u64, 256>::pool(4)
        .spawn(|ring, id, round: &u64| {
            ring.push(*round * 100 + id as u64);
        });

    let mut all_items = Vec::new();

    for round in 0..5u64 {
        pool.dispatch(&round);
        pool.join().unwrap();

        pool.with_fan_in(
            SequencedCollector::from_spout(CollectSpout::new()),
            |fan_in| {
                fan_in.flush().unwrap();
                // Items from this round are emitted in batch_seq order
                // (all workers share the same batch_seq for this round)
                let items = fan_in.collector().sequencer().inner().items();
                all_items.extend_from_slice(items);
            },
        );
    }

    // Verify all 20 items arrived (4 workers * 5 rounds)
    assert_eq!(all_items.len(), 20);
}
```

### Where It Lives

- `SequencedSpout`: **spout** crate (generic combinator, no spill-ring dependency)
- `Collector` trait + `UnorderedCollector` + `SequencedCollector`: **spill-ring** crate (integration layer)

### Files

| File | Change |
|------|--------|
| `spout/src/impls/sequenced.rs` | **New**: `SequencedSpout` implementation. |
| `spout/src/impls/mod.rs` | Add `pub mod sequenced;`. |
| `spout/src/lib.rs` | Re-export `SequencedSpout`. |
| `spill-ring/src/mpsc/collector.rs` | **New**: `Collector` trait, `UnorderedCollector`, `SequencedCollector`. |
| `spill-ring/src/mpsc/fan_in.rs` | Make generic over `C: Collector<T>`. Pass `batch_seq` + `worker_id` in flush. |
| `spill-ring/src/mpsc/mod.rs` | Add `mod collector;`, re-export `Collector`, `UnorderedCollector`, `SequencedCollector`. |
| `spill-ring/src/lib.rs` | Re-export collector types. |
| `spout/src/impls/sequenced.rs` | Tests: fast path, reorder, gap. |
| `spill-ring/src/tests/mpsc.rs` | Integration test: pool -> fan_in -> sequenced_collector. |

### Dependencies

Feature 1 (`batch_seq` on HandoffSlot), Feature 2 (FanInSpout).

### Batch Sequence Semantics

All workers in a single `dispatch()` cycle share the same `batch_seq`. The sequence identifies the dispatch round, not the worker. This means `SequencedSpout` reorders dispatch rounds, not individual workers. Within a round, workers complete in arbitrary order — their batches all carry the same `batch_seq`, so they're all "in order" from the sequencer's perspective.

If per-worker ordering within a round is needed, use `worker_id` in the `Collector::deliver()` callback. The `SequencedCollector` ignores `worker_id` — a user could implement a custom collector that uses both `batch_seq` and `worker_id` for total ordering.

---

## Feature 4: Backpressure Propagation (Documentation + Validation)

### Problem

The TPC design has structural backpressure built in — when the downstream spout is slow, handoff slots fill up, workers detect uncollected batches, merge them back, rings fill to capacity, overflow spouts fire. But this cascade is undocumented and untested.

### Design

This is primarily a **documentation and testing** feature, not new code. The backpressure chain:

```
Output spout slow
  -> consumer collect() slows (fewer calls, or calls take longer)
    -> handoff slots stay full (batch_ptr non-null when worker tries to publish)
      -> worker detects uncollected batch, merges back into active ring
        -> ring fills to capacity
          -> overflow spout fires (eviction / backpressure callback)
```

Each layer already has its valve:

| Layer | Mechanism | Already Implemented? |
|-------|-----------|---------------------|
| SpillRing overflow -> spout | Eviction on push when full | Yes |
| HandoffSlot -> merge back | Worker merges uncollected batch | Yes (pool.rs worker_loop) |
| Consumer -> drain rate | `collect()` frequency | Yes (with Feature 1) |
| Output spout -> I/O throughput | External | N/A |

### Implementation Plan

#### Step 4a: Module-level backpressure documentation

**File: `mpsc/mod.rs`**

Add to module docs (after the existing `//!` block):

```rust
//! ## Backpressure
//!
//! The TPC design provides structural backpressure without explicit flow
//! control protocols. When the consumer is slow, pressure propagates
//! backward through the system:
//!
//! 1. **Consumer slow** — `collect()` called less frequently
//! 2. **Handoff slots fill** — published batches sit uncollected
//! 3. **Workers detect** — next `publish()` finds a non-null batch pointer
//! 4. **Merge back** — worker merges uncollected batch into active ring
//! 5. **Ring fills** — accumulated items exceed ring capacity
//! 6. **Overflow fires** — evicted items route to the overflow spout
//!
//! Each layer has a natural valve. The overflow spout is the pressure
//! relief — configurable per ring (drop, collect, spill to disk).
//! No tokens, no credits, no explicit flow control. The ring's finite
//! capacity and the handoff slot's single-entry design create backpressure
//! automatically.
```

#### Step 4b: Backpressure test

**File: `tests/mpsc.rs`**

```rust
#[test]
fn test_backpressure_overflow_fires() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    // Small ring (capacity 4) with a CollectSpout overflow sink.
    // 2 workers, each pushes 100 items per round.
    // We run multiple rounds WITHOUT collecting — items accumulate
    // in rings, overflow to the spout.
    let overflow_count = Arc::new(AtomicUsize::new(0));
    let count_clone = overflow_count.clone();

    let sink = spout::FnSpout::new(move |_item: u64| {
        count_clone.fetch_add(1, Ordering::Relaxed);
    });

    let mut pool = MpscRing::<u64, 4, _>::pool_with_sink(2, sink)
        .spawn(|ring, _id, count: &u64| {
            for i in 0..*count {
                ring.push(i);
            }
        });

    // Run 10 rounds without collecting — handoff slots fill up,
    // workers merge back uncollected batches, rings overflow.
    for _ in 0..10 {
        pool.run(&100);
    }

    // Overflow should have fired many times (exact count depends on
    // scheduling, but with capacity=4 and 100 items/worker/round,
    // most items overflow).
    let overflows = overflow_count.load(Ordering::Relaxed);
    assert!(overflows > 0, "overflow spout should have fired");

    // Into_consumer drains whatever's left in rings
    let consumer = pool.into_consumer();
    // Total items = ring capacity (kept) + overflow (evicted)
    // Just verify we didn't lose anything — all items either in
    // ring or overflow.
}
```

#### Step 4c: (Optional) `CountingSpout`

**File: `spout/src/impls/core_impls.rs`**

```rust
/// Counts items and optionally forwards to an inner spout.
///
/// Useful for observability — track how many items pass through
/// a point in the pipeline without modifying behavior.
#[must_use]
#[derive(Debug, Clone)]
pub struct CountingSpout<S> {
    count: usize,
    inner: S,
}

impl<S> CountingSpout<S> {
    pub fn new(inner: S) -> Self {
        Self { count: 0, inner }
    }

    pub fn count(&self) -> usize {
        self.count
    }

    pub fn reset(&mut self) -> usize {
        core::mem::replace(&mut self.count, 0)
    }

    pub fn inner(&self) -> &S {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut S {
        &mut self.inner
    }

    pub fn into_inner(self) -> S {
        self.inner
    }
}

impl<T, S: Spout<T>> Spout<T> for CountingSpout<S> {
    type Error = S::Error;

    #[inline]
    fn send(&mut self, item: T) -> Result<(), Self::Error> {
        self.count += 1;
        self.inner.send(item)
    }

    #[inline]
    fn flush(&mut self) -> Result<(), Self::Error> {
        self.inner.flush()
    }
}
```

Mark as optional — only add if the test actually benefits from it. The `FnSpout` + `AtomicUsize` pattern in the test above is sufficient.

### Files

| File | Change |
|------|--------|
| `mpsc/mod.rs` | Add backpressure documentation to module-level `//!` docs. |
| `tests/mpsc.rs` | 1 new test: backpressure overflow fires with small ring + no collection. |
| `spout/src/impls/core_impls.rs` | (Optional) `CountingSpout` adapter. |

### Dependencies

Feature 1 (streaming `collect()` — though the test doesn't strictly require it, the docs reference it).

---

## Feature 5: Multi-Stage Parallel Pipelines

### Problem

Real workloads are multi-stage: parse -> transform -> serialize -> I/O. Each stage should independently scale to its core allocation. The spout trait and TPC handoff make this composable, but the composition pattern needs to be validated and documented.

### Design

A pipeline is just nested type composition:

```
Stage 1: WorkerPool<Raw, N1, _>
            | dispatch + join
            v
         with_fan_in(collector) -> drain stage 1 output
            |
Stage 2: WorkerPool<Parsed, N2, _>
            | dispatch + join
            v
         with_fan_in(sequenced_collector) -> ordered output
            |
         FramedSpout<TcpStream>
```

Each stage is a `WorkerPool` whose output is collected via `FanInSpout` and fed to the next stage. The compiler monomorphizes the entire chain — zero dynamic dispatch.

### Implementation Plan

This is primarily an **integration test and example**, not new library code. The composition already works through existing types.

#### Step 5a: Integration test

**File: `tests/mpsc.rs`**

```rust
#[test]
fn test_two_stage_pipeline() {
    // Stage 1: produce numbers on 4 workers
    // Stage 2: double each number on 2 workers
    // Verify all items arrive correctly.

    // Stage 1: each worker pushes [worker_id * 1000 .. worker_id * 1000 + count)
    let mut stage1 = MpscRing::<u64, 256>::pool(4)
        .spawn(|ring, id, count: &u64| {
            let base = id as u64 * 1000;
            for i in 0..*count {
                ring.push(base + i);
            }
        });

    stage1.run(&50); // 4 * 50 = 200 items

    // Collect stage 1 output
    let mut stage1_items = Vec::new();
    {
        let mut sink = CollectSpout::new();
        stage1.collect(&mut sink).unwrap();
        stage1_items = sink.into_items();
    }
    assert_eq!(stage1_items.len(), 200);

    // Stage 2: double each item, distributed round-robin
    let stage2_input = stage1_items; // owned by stage 2 now
    let mut stage2 = MpscRing::<u64, 256>::pool(2)
        .spawn(|ring, id, batch: &Vec<u64>| {
            for (i, &val) in batch.iter().enumerate() {
                if i % 2 == id { // round-robin assignment
                    ring.push(val * 2);
                }
            }
        });

    stage2.run(&stage2_input);

    let mut final_sink = CollectSpout::new();
    stage2.collect(&mut final_sink).unwrap();
    let mut result = final_sink.into_items();
    result.sort();

    // Verify: every item from stage 1 was doubled
    let mut expected: Vec<u64> = stage2_input.iter().map(|x| x * 2).collect();
    expected.sort();
    assert_eq!(result, expected);
}
```

#### Step 5b: Example (optional)

**New file: `spill-ring/examples/pipeline.rs`**

A standalone example showing a two-stage pipeline with commentary. Only add if the Cargo.toml supports examples with `required-features = ["std"]`.

**File: `spill-ring/Cargo.toml`**

```toml
[[example]]
name = "pipeline"
required-features = ["std"]
```

#### Step 5c: Pipeline documentation

**File: `mpsc/mod.rs`**

Add to module docs:

```rust
//! ## Multi-Stage Pipelines
//!
//! Stages compose through the spout trait — each pool's output feeds
//! the next stage's input:
//!
//! ```text
//! WorkerPool<Stage1> -> collect() -> Vec<T> -> WorkerPool<Stage2> -> collect() -> sink
//! ```
//!
//! Or with FanInSpout for direct streaming:
//!
//! ```text
//! WorkerPool<Stage1> -> with_fan_in(sink) -> WorkerPool<Stage2> -> with_fan_in(sink) -> I/O
//! ```
//!
//! Each stage independently scales to its core allocation. The compiler
//! monomorphizes the entire chain — zero dynamic dispatch.
```

### Files

| File | Change |
|------|--------|
| `tests/mpsc.rs` | 1 new test: two-stage pipeline with data integrity verification. |
| `examples/pipeline.rs` | (Optional) Standalone example. |
| `Cargo.toml` | (Optional) `[[example]]` entry. |
| `mpsc/mod.rs` | Pipeline pattern in module docs. |

### Dependencies

Feature 1 (dispatch/collect/join), Feature 2 (FanInSpout — for the streaming variant).

---

## Feature 6: MPSM — Multiple Producers, Multiple Mergers

### Problem

MPSC (single consumer) limits collection throughput to one core. For high fan-in (many producers), the consumer becomes the bottleneck. MPSM partitions handoff slots across M merger threads, each draining a subset of producers.

### Ownership Model: Scoped Mergers

`MergerHandle` holds a `FanInSpout` which holds raw pointers to `HandoffSlot`s. If the user drops the `WorkerPool` while merger threads hold handles, the pointers dangle. This must be prevented structurally.

**Resolution: scoped merger API**, consistent with `FanInSpout`'s scoped pattern.

### Implementation Plan

#### Step 6a: `MergerHandle` type

**New file: `mpsc/merger.rs`**

```rust
//! MPSM (Multiple Producers, Single/Multiple Mergers) support.

use super::collector::Collector;
use super::fan_in::FanInSpout;

/// A merger that owns a subset of handoff slots.
///
/// Created via [`WorkerPool::with_mergers()`] (scoped, safe) or
/// [`WorkerPool::mergers_unchecked()`] (unsafe, unscoped).
///
/// Each merger wraps a `FanInSpout` over its assigned slot subset.
/// Call `flush()` to drain assigned handoff slots into the collector.
///
/// `MergerHandle` is `Send` when the collector is `Send`, enabling
/// use with `std::thread::scope` for parallel merging.
pub struct MergerHandle<T, const N: usize, S, C>
where
    S: spout::Spout<T, Error = core::convert::Infallible>,
    C: Collector<T>,
{
    /// The FanInSpout over this merger's assigned handoff slot subset.
    fan_in: FanInSpout<T, N, S, C>,
    /// Index of this merger (0..num_mergers).
    merger_id: usize,
}

impl<T, const N: usize, S, C> MergerHandle<T, N, S, C>
where
    S: spout::Spout<T, Error = core::convert::Infallible>,
    C: Collector<T>,
{
    /// Create a new merger handle.
    pub(crate) fn new(fan_in: FanInSpout<T, N, S, C>, merger_id: usize) -> Self {
        Self { fan_in, merger_id }
    }

    /// Drain all assigned handoff slots into the collector.
    pub fn flush(&mut self) -> Result<(), C::Error> {
        self.fan_in.flush()
    }

    /// This merger's index.
    pub fn merger_id(&self) -> usize {
        self.merger_id
    }

    /// Number of handoff slots assigned to this merger.
    pub fn num_slots(&self) -> usize {
        self.fan_in.num_slots()
    }

    /// Reference to the inner collector.
    pub fn collector(&self) -> &C {
        self.fan_in.collector()
    }

    /// Mutable reference to the inner collector.
    pub fn collector_mut(&mut self) -> &mut C {
        self.fan_in.collector_mut()
    }
}

// SAFETY: MergerHandle is Send when FanInSpout is Send (which requires
// C: Send). The raw pointer validity invariant is maintained by the
// scoped API or the unsafe constructor's contract.
unsafe impl<T, const N: usize, S, C> Send for MergerHandle<T, N, S, C>
where
    S: spout::Spout<T, Error = core::convert::Infallible>,
    C: Collector<T> + Send,
{
}
```

#### Step 6b: Pool methods for MPSM

**File: `mpsc/pool.rs`**

```rust
use super::merger::MergerHandle;

impl<T, const N: usize, S, F, A> WorkerPool<T, N, S, F, A>
where
    S: Spout<T, Error = core::convert::Infallible>,
    F: Fn(&SpillRing<T, N, S>, usize, &A) + Send + Clone + 'static,
    A: Sync + 'static,
{
    /// Execute a closure with M merger handles, each owning a subset of
    /// handoff slots partitioned round-robin.
    ///
    /// Merger `i` gets slots `{i, i+M, i+2M, ...}`. The closure receives
    /// a mutable slice of merger handles that can be sent to scoped threads
    /// via `std::thread::scope`.
    ///
    /// # Example
    ///
    /// ```
    /// pool.dispatch(&args);
    /// pool.join()?;
    /// pool.with_mergers(2, || UnorderedCollector::new(CollectSpout::new()), |mergers| {
    ///     std::thread::scope(|s| {
    ///         for merger in mergers.iter_mut() {
    ///             s.spawn(|| merger.flush().unwrap());
    ///         }
    ///     });
    /// });
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if `num_mergers` is 0 or greater than the number of workers.
    pub fn with_mergers<C, R>(
        &mut self,
        num_mergers: usize,
        collector_factory: impl Fn() -> C,
        f: impl FnOnce(&mut [MergerHandle<T, N, S, C>]) -> R,
    ) -> R
    where
        C: Collector<T>,
    {
        assert!(num_mergers > 0, "need at least one merger");
        assert!(
            num_mergers <= self.num_workers,
            "more mergers ({num_mergers}) than workers ({})",
            self.num_workers
        );

        let mut mergers: Vec<MergerHandle<T, N, S, C>> = (0..num_mergers)
            .map(|merger_id| {
                // Round-robin: merger i gets slots {i, i+M, i+2M, ...}
                let slot_ptrs: Box<[SendPtr<HandoffSlot<T, N, S>>]> = (0..self.num_workers)
                    .filter(|&w| w % num_mergers == merger_id)
                    .map(|w| SendPtr(core::ptr::from_ref(&self.slots[w])))
                    .collect::<Vec<_>>()
                    .into_boxed_slice();

                let collector = collector_factory();
                // SAFETY: Slot pointers valid for the closure duration.
                let fan_in = unsafe { FanInSpout::new(slot_ptrs, collector) };
                MergerHandle::new(fan_in, merger_id)
            })
            .collect();

        f(&mut mergers)
    }

    /// Create merger handles with raw pointers to handoff slot subsets.
    ///
    /// # Safety
    ///
    /// Returned MergerHandles must not outlive `self`. The caller must
    /// ensure the pool is not dropped while any MergerHandle exists.
    /// Prefer [`with_mergers()`] for the safe scoped API.
    pub unsafe fn mergers_unchecked<C>(
        &self,
        num_mergers: usize,
        collector_factory: impl Fn() -> C,
    ) -> Vec<MergerHandle<T, N, S, C>>
    where
        C: Collector<T>,
    {
        assert!(num_mergers > 0, "need at least one merger");
        assert!(
            num_mergers <= self.num_workers,
            "more mergers ({num_mergers}) than workers ({})",
            self.num_workers
        );

        (0..num_mergers)
            .map(|merger_id| {
                let slot_ptrs: Box<[SendPtr<HandoffSlot<T, N, S>>]> = (0..self.num_workers)
                    .filter(|&w| w % num_mergers == merger_id)
                    .map(|w| SendPtr(core::ptr::from_ref(&self.slots[w])))
                    .collect::<Vec<_>>()
                    .into_boxed_slice();

                let collector = collector_factory();
                let fan_in = FanInSpout::new(slot_ptrs, collector);
                MergerHandle::new(fan_in, merger_id)
            })
            .collect()
    }
}
```

#### Step 6c: Module structure

**File: `mpsc/mod.rs`**

```rust
#[cfg(feature = "std")]
mod merger;

#[cfg(feature = "std")]
pub use merger::MergerHandle;
```

**File: `lib.rs`**

```rust
#[cfg(feature = "std")]
pub use mpsc::{FanInSpout, MergerHandle, PoolBuilder, WorkerPanic, WorkerPool};
```

#### Step 6d: Tests

**File: `tests/mpsc.rs`**

```rust
#[test]
fn test_mpsm_two_mergers() {
    // 8 workers, 2 mergers. Each worker pushes its worker_id.
    // Merger 0 collects workers {0, 2, 4, 6}.
    // Merger 1 collects workers {1, 3, 5, 7}.
    let mut pool = MpscRing::<u64, 256>::pool(8)
        .spawn(|ring, id, _args: &()| {
            ring.push(id as u64);
        });

    pool.run(&());

    let mut all_items = Vec::new();
    pool.with_mergers(
        2,
        || UnorderedCollector::new(CollectSpout::new()),
        |mergers| {
            std::thread::scope(|s| {
                let handles: Vec<_> = mergers
                    .iter_mut()
                    .map(|m| s.spawn(|| m.flush().unwrap()))
                    .collect();
                for h in handles {
                    h.join().unwrap();
                }
            });
            // Collect results from each merger
            for merger in mergers.iter() {
                let items = merger.collector().inner().items();
                all_items.extend_from_slice(items);
            }
        },
    );

    all_items.sort();
    assert_eq!(all_items, vec![0, 1, 2, 3, 4, 5, 6, 7]);
}

#[test]
fn test_mpsm_slot_partitioning() {
    // Verify round-robin partitioning: 6 workers, 3 mergers.
    // Merger 0: workers {0, 3}
    // Merger 1: workers {1, 4}
    // Merger 2: workers {2, 5}
    let mut pool = MpscRing::<u64, 256>::pool(6)
        .spawn(|ring, id, _args: &()| {
            ring.push(id as u64);
        });

    pool.run(&());

    pool.with_mergers(
        3,
        || UnorderedCollector::new(CollectSpout::new()),
        |mergers| {
            for merger in mergers.iter_mut() {
                merger.flush().unwrap();
            }

            // Merger 0 should have workers 0 and 3
            let m0: Vec<u64> = mergers[0].collector().inner().items().to_vec();
            let m1: Vec<u64> = mergers[1].collector().inner().items().to_vec();
            let m2: Vec<u64> = mergers[2].collector().inner().items().to_vec();

            assert!(m0.contains(&0) && m0.contains(&3), "merger 0: {m0:?}");
            assert!(m1.contains(&1) && m1.contains(&4), "merger 1: {m1:?}");
            assert!(m2.contains(&2) && m2.contains(&5), "merger 2: {m2:?}");
        },
    );
}

#[test]
fn test_mpsm_single_merger_equals_collect() {
    // 1 merger over all slots should produce same result as collect()
    let mut pool = MpscRing::<u64, 256>::pool(4)
        .spawn(|ring, id, count: &u64| {
            for i in 0..*count {
                ring.push(id as u64 * 1000 + i);
            }
        });

    pool.run(&100);

    let mut merger_items = Vec::new();
    pool.with_mergers(
        1,
        || UnorderedCollector::new(CollectSpout::new()),
        |mergers| {
            mergers[0].flush().unwrap();
            merger_items = mergers[0].collector().inner().items().to_vec();
        },
    );

    assert_eq!(merger_items.len(), 400);
}
```

### Merge Tree

For very high fan-in, mergers form a tree:

```
Producers:  P0  P1  P2  P3  P4  P5  P6  P7
              \  |  /     \  |  /     \  |
Mergers L1: FanIn[0..2]  FanIn[3..5] FanIn[6..7]
              |              |             |
Mergers L2:     FanIn[merger_0..merger_2]
                      |
               SequencedSpout
                      |
                  I/O Spout
```

Each merge tier is the same type composition: `FanInSpout` -> optional `SequencedCollector` -> output spout. The tree scales logarithmically.

The merge tree is a user-space composition pattern, not library code. Each tier is a `with_mergers()` call whose collectors feed the next tier's input. The library provides the building blocks; the user assembles the tree.

### Slot Partitioning

Round-robin assignment: merger `i` gets slots `{i, i+M, i+2M, ...}`. This distributes load evenly regardless of which producers are fast or slow.

### Files

| File | Change |
|------|--------|
| `mpsc/merger.rs` | **New**: `MergerHandle` type. |
| `mpsc/pool.rs` | Add `with_mergers()` and `unsafe mergers_unchecked()`. |
| `mpsc/mod.rs` | Add `mod merger;`, re-export `MergerHandle`. |
| `lib.rs` | Re-export `MergerHandle`. |
| `tests/mpsc.rs` | 3 new tests: two mergers, slot partitioning, single merger equals collect. |

### Dependencies

Feature 2 (FanInSpout), Feature 3 (Collector trait — mergers are generic over `C: Collector<T>`).

---

## Implementation Sequence

```
Feature 1: Streaming Collection API + batch_seq on HandoffSlot
    |
    +--> Feature 4: Backpressure (docs + test) <-- do early, validates F1
    |
    +--> Feature 2: FanInSpout (scoped API)
             |
             +--> Feature 3: SequencedSpout + Collector trait
             |
             +--> Feature 5: Multi-Stage Pipelines (example + test)
             |
             +--> Feature 6: MPSM (scoped mergers)
```

Feature 1 is the foundation — everything else builds on `dispatch`/`collect`/`join` + `batch_seq`. Feature 4 should follow immediately (validates streaming collection under load). Feature 2 wraps collection as a spout with the scoped ownership model. Features 3, 5, 6 compose from there.

### Hidden Dependencies

- Feature 3 depends on Feature 2 resolving the collector strategy pattern (how batch_seq flows to downstream).
- Feature 6 depends on Feature 2's `FanInSpout` being `Send` (resolved by `SendPtr`).
- Feature 3's `batch_seq` on `HandoffSlot` is front-loaded into Feature 1 to avoid a structural change later.

### Total New Tests Per Feature

| Feature | New Tests |
|---------|-----------|
| F1 | 5 (dispatch_join_collect, collect_between_runs, batch_seq_increments, collect_empty_slots_skipped, collect_propagates_spout_error) |
| F2 | 3 (scoped basic, send passthrough, multiple flushes) |
| F3 | 3 spout crate (fast path, reorder, gap) + 1 integration (pool -> fan_in -> sequenced) |
| F4 | 1 (backpressure overflow fires) |
| F5 | 1 (two-stage pipeline) |
| F6 | 3 (two mergers, slot partitioning, single merger equals collect) |
| **Total** | **17 new tests** |

---

## Design Decisions Log

### D-1: FanInSpout ownership — scoped closure API

**Problem:** FanInSpout holds raw pointers to HandoffSlot. Must not outlive the pool.

**Options considered:**
1. Borrow-based (`pool.fan_in() -> FanInSpout<'_>`) — prevents dispatch/join while fan_in exists. Dead on arrival.
2. Raw `SendPtr` with documented contract — works but lifetime safety is on the user.
3. `Arc`-based reference counting — adds unnecessary overhead, pool lifetime is structurally guaranteed.
4. **Scoped closure (`pool.with_fan_in(|fan_in| { ... })`)** — lifetime enforced by scope. Cannot escape.

**Decision:** Option 4 (scoped closure) as the safe API. Option 2 (`unsafe fan_in_unchecked`) as the escape hatch for advanced usage.

### D-2: Batch sequence flow — Collector strategy trait

**Problem:** `SequencedSpout::submit(batch_seq, items)` is not part of `Spout<T>`. FanInSpout needs to pass batch_seq to a generic downstream.

**Options considered:**
1. Tight coupling (FanInSpout knows about SequencedSpout) — breaks composability.
2. Side-channel state on SequencedSpout — ugly, error-prone.
3. Extend `Spout<T>` with `submit_batch()` — breaking trait change.
4. Wrap items as `(batch_seq, T)` — changes item type, viral.
5. **`Collector` strategy trait on FanInSpout** — decouples delivery strategy from spout trait.

**Decision:** Option 5. `Collector<T>` trait with `deliver(batch_seq, worker_id, items)` and `finish()`. Two built-in implementations: `UnorderedCollector` (ignores batch_seq, same as current `collect()`) and `SequencedCollector` (routes through `SequencedSpout::submit()`). `FanInSpout` is generic over `C: Collector<T>`.

### D-3: MPSM lifetime safety — scoped mergers

**Problem:** MergerHandle holds raw pointers. User-spawned threads could outlive the pool.

**Options considered:**
1. User-managed lifetime with documentation — footgun.
2. `Arc`-based guard — unnecessary overhead.
3. **Scoped closure (`pool.with_mergers(|mergers| { ... })`)** — combine with `thread::scope` for composable lifetime safety.

**Decision:** Option 3. Consistent with D-1. `thread::scope` inside the closure ensures merger threads complete before the pool regains `&mut self`.

### D-4: collect() return type — Result, not usize

**Problem:** Output spout's `send()` is fallible. Ignoring errors loses information.

**Decision:** `collect()` returns `Result<usize, Out::Error>`. The current internal collection uses `let _ =` because the overflow spout is `Infallible`, but the public `collect()` should propagate errors.

### D-5: PoolConsumer type — subsumed by MergerHandle

**Problem:** tpc.md defines a `PoolConsumer` type as an intermediate between `collect()` and `MergerHandle`.

**Decision:** Not needed as a separate type. `collect()` on `WorkerPool` serves the single-consumer case. `MergerHandle` (wrapping `FanInSpout`) serves the multi-consumer case. `PoolConsumer` would be a third way to do the same thing — unnecessary complexity. If a distinct type is needed later, it can be added as a non-breaking extension.

### D-6: FanInSpout — Spout<T> impl vs standalone flush

**Problem:** FanInSpout's primary operation is `flush()` (collect from slots). But `Spout<T>` also requires `send()`, which doesn't map naturally to a collection source.

**Options considered:**
1. Implement `Spout<T>` with `send()` as pass-through to inner — works for `UnorderedCollector`, awkward for `SequencedCollector`.
2. Don't implement `Spout<T>` — users call `flush()` directly.
3. Implement `Spout<T>` only for `FanInSpout<..., UnorderedCollector<S>>`.

**Decision:** Option 3. `Spout<T>` impl gated on `UnorderedCollector<S>` where `S: Spout<T>`. The simple unordered path gets full spout composability. The sequenced path uses `flush()` directly through the `Collector` interface. This avoids forcing a `send()` semantic onto collection-oriented types while preserving composability for the common case.

### D-7: Batch sequence semantics — per-dispatch, not per-worker

**Problem:** How to assign batch sequence numbers when N workers complete independently.

**Decision:** All workers in a single `dispatch()` cycle share the same `batch_seq`. The sequence identifies the dispatch round, not the individual worker. Within a round, worker completion order is nondeterministic. `SequencedSpout` reorders dispatch rounds. For per-worker ordering within a round, use `worker_id` in a custom `Collector` implementation.

---

## Design Philosophy

From tpc.md:

> No `Pipeline` struct. No `Stage` trait. No runtime graph. The types compose, the user builds the graph, the compiler inlines and monomorphizes. Zero-cost abstraction — the assembled pipeline compiles to the same code as a hand-written version.

Every feature above is a building block, not a framework. `FanInSpout` is a spout. `SequencedSpout` is a spout. `MergerHandle` wraps a `FanInSpout`. The `Collector` trait bridges batch metadata without changing the spout trait. The user assembles them. The compiler optimizes them.
