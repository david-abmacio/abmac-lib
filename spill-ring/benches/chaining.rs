//! Ring chaining benchmarks — `push(&self)` path throughout.
//!
//! Chained rings can only use `push(&self)` because the inner ring is owned
//! by the outer ring's spout (no `&mut self` available). Single-ring baselines
//! also use `push(&self)` so both sides go through the same code path,
//! making the comparison fair.
//!
//! NOTE: Chained rings must be recreated each iteration because inner rings
//! are owned by the outer ring's spout — there's no way to reset the full
//! chain without rebuilding it.

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use spill_ring::SpillRing;
use spout::{BatchSpout, CollectSpout, DropSpout, ReduceSpout};
use std::hint::black_box;

/// Compare single ring vs chained rings — no overflow case.
/// When items stay in the first ring, chaining should have zero overhead.
fn chaining_no_overflow(c: &mut Criterion) {
    let mut group = c.benchmark_group("chaining/push/no_overflow");

    let iterations = 1000u64;
    group.throughput(Throughput::Elements(iterations));

    // Single ring, capacity 64 — push(&self) to match chained path
    // Must recreate each iteration (no clear() through &self)
    group.bench_function("single_ring_64", |b| {
        b.iter(|| {
            let ring: SpillRing<u64, 64> = SpillRing::new();
            for i in 0..32u64 {
                ring.push(black_box(i));
            }
            black_box(ring.len())
        })
    });

    // Chained: ring1(32) -> ring2(32) - push 32 items (no overflow from ring1)
    // Must recreate each iteration — inner ring owned by outer spout
    group.bench_function("chained_32_32", |b| {
        b.iter(|| {
            let ring2: SpillRing<u64, 32> = SpillRing::new();
            let ring1 = SpillRing::<u64, 32, _>::builder().spout(ring2).build();
            for i in 0..32u64 {
                ring1.push(black_box(i));
            }
            black_box(ring1.len())
        })
    });

    group.finish();
}

/// Compare single ring vs chained rings — with overflow.
/// Measures the cost of cascading items through the chain.
fn chaining_with_overflow(c: &mut Criterion) {
    let mut group = c.benchmark_group("chaining/push/with_overflow");

    let iterations = 10_000u64;
    group.throughput(Throughput::Elements(iterations));

    // Single ring capacity 64 — push(&self) to match chained path
    group.bench_function("single_64_to_drop", |b| {
        b.iter(|| {
            let ring: SpillRing<u64, 64, DropSpout> = SpillRing::new();
            for i in 0..iterations {
                ring.push(black_box(i));
            }
            black_box(ring.len())
        })
    });

    // Chained: ring1(32) -> ring2(32) -> DropSpout, push 10k items
    group.bench_function("chained_32_32_to_drop", |b| {
        b.iter(|| {
            let ring2: SpillRing<u64, 32, DropSpout> = SpillRing::new();
            let ring1 = SpillRing::<u64, 32, _>::builder().spout(ring2).build();
            for i in 0..iterations {
                ring1.push(black_box(i));
            }
            black_box(ring1.len())
        })
    });

    // Three-level chain: ring1(16) -> ring2(16) -> ring3(32) -> DropSpout
    group.bench_function("chained_16_16_32_to_drop", |b| {
        b.iter(|| {
            let ring3: SpillRing<u64, 32, DropSpout> = SpillRing::new();
            let ring2 = SpillRing::<u64, 16, _>::builder().spout(ring3).build();
            let ring1 = SpillRing::<u64, 16, _>::builder().spout(ring2).build();
            for i in 0..iterations {
                ring1.push(black_box(i));
            }
            black_box(ring1.len())
        })
    });

    group.finish();
}

/// Chaining with collection — measure end-to-end with data preservation.
fn chaining_collect(c: &mut Criterion) {
    let mut group = c.benchmark_group("chaining/push/collect");

    let iterations = 10_000u64;
    group.throughput(Throughput::Elements(iterations));

    // Single ring -> CollectSpout — push(&self) to match chained path
    group.bench_function("single_64_collect", |b| {
        b.iter(|| {
            let ring: SpillRing<u64, 64, _> = SpillRing::builder().spout(CollectSpout::new()).build();
            for i in 0..iterations {
                ring.push(black_box(i));
            }
            black_box(ring.len())
        })
    });

    // Chained: ring1(32) -> ring2(32) -> CollectSpout
    group.bench_function("chained_32_32_collect", |b| {
        b.iter(|| {
            let ring2: SpillRing<u64, 32, _> = SpillRing::builder().spout(CollectSpout::new()).build();
            let ring1 = SpillRing::<u64, 32, _>::builder().spout(ring2).build();
            for i in 0..iterations {
                ring1.push(black_box(i));
            }
            black_box(ring1.len())
        })
    });

    group.finish();
}

/// Varying chain depths — how does overhead scale?
fn chaining_depth(c: &mut Criterion) {
    let mut group = c.benchmark_group("chaining/push/depth");

    let iterations = 10_000u64;
    group.throughput(Throughput::Elements(iterations));

    // Depth 1: single ring — push(&self) to match chained path
    group.bench_function("depth_1", |b| {
        b.iter(|| {
            let ring: SpillRing<u64, 64, DropSpout> = SpillRing::new();
            for i in 0..iterations {
                ring.push(black_box(i));
            }
            black_box(ring.len())
        })
    });

    // Depth 2: ring -> ring -> drop
    group.bench_function("depth_2", |b| {
        b.iter(|| {
            let r2: SpillRing<u64, 32, DropSpout> = SpillRing::new();
            let r1 = SpillRing::<u64, 32, _>::builder().spout(r2).build();
            for i in 0..iterations {
                r1.push(black_box(i));
            }
            black_box(r1.len())
        })
    });

    // Depth 3: ring -> ring -> ring -> drop
    group.bench_function("depth_3", |b| {
        b.iter(|| {
            let r3: SpillRing<u64, 32, DropSpout> = SpillRing::new();
            let r2 = SpillRing::<u64, 16, _>::builder().spout(r3).build();
            let r1 = SpillRing::<u64, 16, _>::builder().spout(r2).build();
            for i in 0..iterations {
                r1.push(black_box(i));
            }
            black_box(r1.len())
        })
    });

    // Depth 4: ring -> ring -> ring -> ring -> drop
    group.bench_function("depth_4", |b| {
        b.iter(|| {
            let r4: SpillRing<u64, 16, DropSpout> = SpillRing::new();
            let r3 = SpillRing::<u64, 16, _>::builder().spout(r4).build();
            let r2 = SpillRing::<u64, 16, _>::builder().spout(r3).build();
            let r1 = SpillRing::<u64, 16, _>::builder().spout(r2).build();
            for i in 0..iterations {
                r1.push(black_box(i));
            }
            black_box(r1.len())
        })
    });

    group.finish();
}

/// BatchSpout and ReduceSpout — cutting cascade overhead.
fn batch_reduce_spouts(c: &mut Criterion) {
    let mut group = c.benchmark_group("chaining/push/batch_reduce");

    let iterations = 10_000u64;
    group.throughput(Throughput::Elements(iterations));

    // Baseline: depth 4 chain without batching
    group.bench_function("depth_4_no_batch", |b| {
        b.iter(|| {
            let r4: SpillRing<u64, 16, DropSpout> = SpillRing::new();
            let r3 = SpillRing::<u64, 16, _>::builder().spout(r4).build();
            let r2 = SpillRing::<u64, 16, _>::builder().spout(r3).build();
            let r1 = SpillRing::<u64, 16, _>::builder().spout(r2).build();
            for i in 0..iterations {
                r1.push(black_box(i));
            }
            black_box(r1.len())
        })
    });

    // With BatchSpout after ring2 - batches of 100 cut traffic to r3/r4
    group.bench_function("depth_4_with_batch_100", |b| {
        b.iter(|| {
            let r4: SpillRing<Vec<u64>, 16, DropSpout> = SpillRing::new();
            let r3 = SpillRing::<Vec<u64>, 16, _>::builder().spout(r4).build();
            let batch_spout: BatchSpout<u64, _> = BatchSpout::new(100, r3);
            let r2 = SpillRing::<u64, 16, _>::builder().spout(batch_spout).build();
            let r1 = SpillRing::<u64, 16, _>::builder().spout(r2).build();
            for i in 0..iterations {
                r1.push(black_box(i));
            }
            black_box(r1.len())
        })
    });

    // ReduceSpout — sum batches of 100
    group.bench_function("reduce_sum_100", |b| {
        b.iter(|| {
            let collect: CollectSpout<u64> = CollectSpout::new();
            let reduce: ReduceSpout<u64, u64, _, _> =
                ReduceSpout::new(100, |batch: Vec<u64>| batch.iter().sum(), collect);
            let ring = SpillRing::<u64, 64, _>::builder().spout(reduce).build();
            for i in 0..iterations {
                ring.push(black_box(i));
            }
            black_box(ring.len())
        })
    });

    // ReduceSpout — count batches (lighter reduce function)
    group.bench_function("reduce_count_100", |b| {
        b.iter(|| {
            let collect: CollectSpout<usize> = CollectSpout::new();
            let reduce: ReduceSpout<u64, usize, _, _> =
                ReduceSpout::new(100, |batch: Vec<u64>| batch.len(), collect);
            let ring = SpillRing::<u64, 64, _>::builder().spout(reduce).build();
            for i in 0..iterations {
                ring.push(black_box(i));
            }
            black_box(ring.len())
        })
    });

    // BatchSpout alone — just batching overhead
    group.bench_function("batch_only_100", |b| {
        b.iter(|| {
            let collect: CollectSpout<Vec<u64>> = CollectSpout::new();
            let batch: BatchSpout<u64, _> = BatchSpout::new(100, collect);
            let ring = SpillRing::<u64, 64, _>::builder().spout(batch).build();
            for i in 0..iterations {
                ring.push(black_box(i));
            }
            black_box(ring.len())
        })
    });

    // Baseline: no batching, just CollectSpout
    group.bench_function("no_batch_collect", |b| {
        b.iter(|| {
            let collect: CollectSpout<u64> = CollectSpout::new();
            let ring = SpillRing::<u64, 64, _>::builder().spout(collect).build();
            for i in 0..iterations {
                ring.push(black_box(i));
            }
            black_box(ring.len())
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    chaining_no_overflow,
    chaining_with_overflow,
    chaining_collect,
    chaining_depth,
    batch_reduce_spouts,
);
criterion_main!(benches);
