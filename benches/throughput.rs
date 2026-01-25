//! Throughput benchmarks - ops/sec for push and pop operations.

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use spill_ring::{CollectSink, DropSink, SpillRing};

/// Benchmark push throughput with DropSink (items discarded on eviction).
fn push_throughput_drop_sink(c: &mut Criterion) {
    let mut group = c.benchmark_group("push_throughput_drop_sink");

    for capacity in [16, 64, 256, 1024] {
        group.throughput(Throughput::Elements(10_000));
        group.bench_with_input(
            BenchmarkId::from_parameter(capacity),
            &capacity,
            |b, &cap| match cap {
                16 => b.iter(|| {
                    let ring: SpillRing<u64, 16, DropSink> = SpillRing::new();
                    for i in 0..10_000u64 {
                        ring.push(black_box(i));
                    }
                }),
                64 => b.iter(|| {
                    let ring: SpillRing<u64, 64, DropSink> = SpillRing::new();
                    for i in 0..10_000u64 {
                        ring.push(black_box(i));
                    }
                }),
                256 => b.iter(|| {
                    let ring: SpillRing<u64, 256, DropSink> = SpillRing::new();
                    for i in 0..10_000u64 {
                        ring.push(black_box(i));
                    }
                }),
                1024 => b.iter(|| {
                    let ring: SpillRing<u64, 1024, DropSink> = SpillRing::new();
                    for i in 0..10_000u64 {
                        ring.push(black_box(i));
                    }
                }),
                _ => unreachable!(),
            },
        );
    }
    group.finish();
}

/// Benchmark push throughput with CollectSink (items collected on eviction).
fn push_throughput_collect_sink(c: &mut Criterion) {
    let mut group = c.benchmark_group("push_throughput_collect_sink");

    for capacity in [16, 64, 256] {
        group.throughput(Throughput::Elements(10_000));
        group.bench_with_input(
            BenchmarkId::from_parameter(capacity),
            &capacity,
            |b, &cap| match cap {
                16 => b.iter(|| {
                    let ring: SpillRing<u64, 16, _> = SpillRing::with_sink(CollectSink::new());
                    for i in 0..10_000u64 {
                        ring.push(black_box(i));
                    }
                }),
                64 => b.iter(|| {
                    let ring: SpillRing<u64, 64, _> = SpillRing::with_sink(CollectSink::new());
                    for i in 0..10_000u64 {
                        ring.push(black_box(i));
                    }
                }),
                256 => b.iter(|| {
                    let ring: SpillRing<u64, 256, _> = SpillRing::with_sink(CollectSink::new());
                    for i in 0..10_000u64 {
                        ring.push(black_box(i));
                    }
                }),
                _ => unreachable!(),
            },
        );
    }
    group.finish();
}

/// Benchmark pop throughput.
fn pop_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("pop_throughput");

    for capacity in [16, 64, 256, 1024] {
        group.throughput(Throughput::Elements(capacity as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(capacity),
            &capacity,
            |b, &cap| match cap {
                16 => b.iter(|| {
                    let ring: SpillRing<u64, 16> = SpillRing::new();
                    for i in 0..16u64 {
                        ring.push(i);
                    }
                    for _ in 0..16 {
                        black_box(ring.pop());
                    }
                }),
                64 => b.iter(|| {
                    let ring: SpillRing<u64, 64> = SpillRing::new();
                    for i in 0..64u64 {
                        ring.push(i);
                    }
                    for _ in 0..64 {
                        black_box(ring.pop());
                    }
                }),
                256 => b.iter(|| {
                    let ring: SpillRing<u64, 256> = SpillRing::new();
                    for i in 0..256u64 {
                        ring.push(i);
                    }
                    for _ in 0..256 {
                        black_box(ring.pop());
                    }
                }),
                1024 => b.iter(|| {
                    let ring: SpillRing<u64, 1024> = SpillRing::new();
                    for i in 0..1024u64 {
                        ring.push(i);
                    }
                    for _ in 0..1024 {
                        black_box(ring.pop());
                    }
                }),
                _ => unreachable!(),
            },
        );
    }
    group.finish();
}

/// Benchmark push+pop interleaved (realistic usage pattern).
fn push_pop_interleaved(c: &mut Criterion) {
    let mut group = c.benchmark_group("push_pop_interleaved");

    for capacity in [16, 64, 256] {
        group.throughput(Throughput::Elements(10_000));
        group.bench_with_input(
            BenchmarkId::from_parameter(capacity),
            &capacity,
            |b, &cap| match cap {
                16 => b.iter(|| {
                    let ring: SpillRing<u64, 16> = SpillRing::new();
                    for i in 0..10_000u64 {
                        ring.push(black_box(i));
                        if i % 2 == 0 {
                            black_box(ring.pop());
                        }
                    }
                }),
                64 => b.iter(|| {
                    let ring: SpillRing<u64, 64> = SpillRing::new();
                    for i in 0..10_000u64 {
                        ring.push(black_box(i));
                        if i % 2 == 0 {
                            black_box(ring.pop());
                        }
                    }
                }),
                256 => b.iter(|| {
                    let ring: SpillRing<u64, 256> = SpillRing::new();
                    for i in 0..10_000u64 {
                        ring.push(black_box(i));
                        if i % 2 == 0 {
                            black_box(ring.pop());
                        }
                    }
                }),
                _ => unreachable!(),
            },
        );
    }
    group.finish();
}

/// Benchmark with different item sizes.
fn item_size_impact(c: &mut Criterion) {
    let mut group = c.benchmark_group("item_size_impact");

    // u64 - 8 bytes
    group.throughput(Throughput::Elements(10_000));
    group.bench_function("u64_8bytes", |b| {
        b.iter(|| {
            let ring: SpillRing<u64, 64> = SpillRing::new();
            for i in 0..10_000u64 {
                ring.push(black_box(i));
            }
        })
    });

    // [u64; 4] - 32 bytes
    group.bench_function("array_32bytes", |b| {
        b.iter(|| {
            let ring: SpillRing<[u64; 4], 64> = SpillRing::new();
            for i in 0..10_000u64 {
                ring.push(black_box([i, i, i, i]));
            }
        })
    });

    // [u64; 16] - 128 bytes
    group.bench_function("array_128bytes", |b| {
        b.iter(|| {
            let ring: SpillRing<[u64; 16], 64> = SpillRing::new();
            for i in 0..10_000u64 {
                ring.push(black_box([i; 16]));
            }
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    push_throughput_drop_sink,
    push_throughput_collect_sink,
    pop_throughput,
    push_pop_interleaved,
    item_size_impact,
);
criterion_main!(benches);
