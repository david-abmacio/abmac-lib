//! Ring chaining benchmarks.
//!
//! Compares single ring vs chained rings to measure the overhead
//! of tiered buffering.

use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use spill_ring::{BatchSink, CollectSink, DropSink, ReduceSink, SpillRing};

/// Compare single ring vs chained rings - no overflow case.
/// When items stay in the first ring, chaining should have zero overhead.
fn chaining_no_overflow(c: &mut Criterion) {
    let mut group = c.benchmark_group("chaining_no_overflow");

    let iterations = 1000u64;
    group.throughput(Throughput::Elements(iterations));

    // Single ring, capacity 64 - push 32 items (no overflow)
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
    group.bench_function("chained_32_32", |b| {
        b.iter(|| {
            let ring2: SpillRing<u64, 32> = SpillRing::new();
            let ring1 = SpillRing::<u64, 32, _>::with_sink(ring2);
            for i in 0..32u64 {
                ring1.push(black_box(i));
            }
            black_box(ring1.len())
        })
    });

    group.finish();
}

/// Compare single ring vs chained rings - with overflow.
/// Measures the cost of cascading items through the chain.
fn chaining_with_overflow(c: &mut Criterion) {
    let mut group = c.benchmark_group("chaining_with_overflow");

    let iterations = 10_000u64;
    group.throughput(Throughput::Elements(iterations));

    // Single ring capacity 64, push 10k items - heavy eviction to DropSink
    group.bench_function("single_64_to_drop", |b| {
        b.iter(|| {
            let ring: SpillRing<u64, 64, DropSink> = SpillRing::new();
            for i in 0..iterations {
                ring.push(black_box(i));
            }
            black_box(ring.len())
        })
    });

    // Chained: ring1(32) -> ring2(32) -> DropSink, push 10k items
    // Items cascade: ring1 overflows to ring2, ring2 overflows to drop
    group.bench_function("chained_32_32_to_drop", |b| {
        b.iter(|| {
            let ring2: SpillRing<u64, 32, DropSink> = SpillRing::new();
            let ring1 = SpillRing::<u64, 32, _>::with_sink(ring2);
            for i in 0..iterations {
                ring1.push(black_box(i));
            }
            black_box(ring1.len())
        })
    });

    // Three-level chain: ring1(16) -> ring2(16) -> ring3(32) -> DropSink
    group.bench_function("chained_16_16_32_to_drop", |b| {
        b.iter(|| {
            let ring3: SpillRing<u64, 32, DropSink> = SpillRing::new();
            let ring2 = SpillRing::<u64, 16, _>::with_sink(ring3);
            let ring1 = SpillRing::<u64, 16, _>::with_sink(ring2);
            for i in 0..iterations {
                ring1.push(black_box(i));
            }
            black_box(ring1.len())
        })
    });

    group.finish();
}

/// Chaining with collection - measure end-to-end with data preservation.
fn chaining_collect(c: &mut Criterion) {
    let mut group = c.benchmark_group("chaining_collect");

    let iterations = 10_000u64;
    group.throughput(Throughput::Elements(iterations));

    // Single ring -> CollectSink
    group.bench_function("single_64_collect", |b| {
        b.iter(|| {
            let ring: SpillRing<u64, 64, _> = SpillRing::with_sink(CollectSink::new());
            for i in 0..iterations {
                ring.push(black_box(i));
            }
            black_box(ring.len())
        })
    });

    // Chained: ring1(32) -> ring2(32) -> CollectSink
    group.bench_function("chained_32_32_collect", |b| {
        b.iter(|| {
            let ring2: SpillRing<u64, 32, _> = SpillRing::with_sink(CollectSink::new());
            let ring1 = SpillRing::<u64, 32, _>::with_sink(ring2);
            for i in 0..iterations {
                ring1.push(black_box(i));
            }
            black_box(ring1.len())
        })
    });

    group.finish();
}

/// Varying chain depths - how does overhead scale?
fn chaining_depth(c: &mut Criterion) {
    let mut group = c.benchmark_group("chaining_depth");

    let iterations = 10_000u64;
    group.throughput(Throughput::Elements(iterations));

    // Depth 1: single ring
    group.bench_function("depth_1", |b| {
        b.iter(|| {
            let ring: SpillRing<u64, 64, DropSink> = SpillRing::new();
            for i in 0..iterations {
                ring.push(black_box(i));
            }
            black_box(ring.len())
        })
    });

    // Depth 2: ring -> ring -> drop
    group.bench_function("depth_2", |b| {
        b.iter(|| {
            let r2: SpillRing<u64, 32, DropSink> = SpillRing::new();
            let r1 = SpillRing::<u64, 32, _>::with_sink(r2);
            for i in 0..iterations {
                r1.push(black_box(i));
            }
            black_box(r1.len())
        })
    });

    // Depth 3: ring -> ring -> ring -> drop
    group.bench_function("depth_3", |b| {
        b.iter(|| {
            let r3: SpillRing<u64, 32, DropSink> = SpillRing::new();
            let r2 = SpillRing::<u64, 16, _>::with_sink(r3);
            let r1 = SpillRing::<u64, 16, _>::with_sink(r2);
            for i in 0..iterations {
                r1.push(black_box(i));
            }
            black_box(r1.len())
        })
    });

    // Depth 4: ring -> ring -> ring -> ring -> drop
    group.bench_function("depth_4", |b| {
        b.iter(|| {
            let r4: SpillRing<u64, 16, DropSink> = SpillRing::new();
            let r3 = SpillRing::<u64, 16, _>::with_sink(r4);
            let r2 = SpillRing::<u64, 16, _>::with_sink(r3);
            let r1 = SpillRing::<u64, 16, _>::with_sink(r2);
            for i in 0..iterations {
                r1.push(black_box(i));
            }
            black_box(r1.len())
        })
    });

    group.finish();
}

/// BatchSink and ReduceSink - cutting cascade overhead.
fn batch_reduce_sinks(c: &mut Criterion) {
    let mut group = c.benchmark_group("batch_reduce_sinks");

    let iterations = 10_000u64;
    group.throughput(Throughput::Elements(iterations));

    // Baseline: depth 4 chain without batching
    group.bench_function("depth_4_no_batch", |b| {
        b.iter(|| {
            let r4: SpillRing<u64, 16, DropSink> = SpillRing::new();
            let r3 = SpillRing::<u64, 16, _>::with_sink(r4);
            let r2 = SpillRing::<u64, 16, _>::with_sink(r3);
            let r1 = SpillRing::<u64, 16, _>::with_sink(r2);
            for i in 0..iterations {
                r1.push(black_box(i));
            }
            black_box(r1.len())
        })
    });

    // With BatchSink after ring2 - batches of 100 cut traffic to r3/r4
    group.bench_function("depth_4_with_batch_100", |b| {
        b.iter(|| {
            let r4: SpillRing<Vec<u64>, 16, DropSink> = SpillRing::new();
            let r3 = SpillRing::<Vec<u64>, 16, _>::with_sink(r4);
            let batch_sink: BatchSink<u64, _> = BatchSink::new(100, r3);
            let r2 = SpillRing::<u64, 16, _>::with_sink(batch_sink);
            let r1 = SpillRing::<u64, 16, _>::with_sink(r2);
            for i in 0..iterations {
                r1.push(black_box(i));
            }
            black_box(r1.len())
        })
    });

    // ReduceSink - sum batches of 100
    group.bench_function("reduce_sum_100", |b| {
        b.iter(|| {
            let collect: CollectSink<u64> = CollectSink::new();
            let reduce: ReduceSink<u64, u64, _, _> =
                ReduceSink::new(100, |batch: Vec<u64>| batch.iter().sum(), collect);
            let ring = SpillRing::<u64, 64, _>::with_sink(reduce);
            for i in 0..iterations {
                ring.push(black_box(i));
            }
            black_box(ring.len())
        })
    });

    // ReduceSink - count batches (lighter reduce function)
    group.bench_function("reduce_count_100", |b| {
        b.iter(|| {
            let collect: CollectSink<usize> = CollectSink::new();
            let reduce: ReduceSink<u64, usize, _, _> =
                ReduceSink::new(100, |batch: Vec<u64>| batch.len(), collect);
            let ring = SpillRing::<u64, 64, _>::with_sink(reduce);
            for i in 0..iterations {
                ring.push(black_box(i));
            }
            black_box(ring.len())
        })
    });

    // BatchSink alone - just batching overhead
    group.bench_function("batch_only_100", |b| {
        b.iter(|| {
            let collect: CollectSink<Vec<u64>> = CollectSink::new();
            let batch: BatchSink<u64, _> = BatchSink::new(100, collect);
            let ring = SpillRing::<u64, 64, _>::with_sink(batch);
            for i in 0..iterations {
                ring.push(black_box(i));
            }
            black_box(ring.len())
        })
    });

    // Baseline: no batching, just CollectSink
    group.bench_function("no_batch_collect", |b| {
        b.iter(|| {
            let collect: CollectSink<u64> = CollectSink::new();
            let ring = SpillRing::<u64, 64, _>::with_sink(collect);
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
    batch_reduce_sinks,
);
criterion_main!(benches);
