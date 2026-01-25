//! Comparison benchmarks - SpillRing vs VecDeque baseline.
//!
//! These benchmarks help developers understand SpillRing's performance
//! relative to standard library alternatives.

use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use spill_ring::SpillRing;
use std::collections::VecDeque;

/// Compare push performance: SpillRing vs VecDeque.
///
/// Note: VecDeque doesn't have automatic eviction, so we manually
/// pop when full to simulate similar behavior.
fn push_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("push_comparison");
    let iterations = 10_000u64;
    group.throughput(Throughput::Elements(iterations));

    // SpillRing - automatic eviction
    group.bench_function("spill_ring_64", |b| {
        b.iter(|| {
            let ring: SpillRing<u64, 64> = SpillRing::new();
            for i in 0..iterations {
                ring.push(black_box(i));
            }
        })
    });

    // VecDeque with manual eviction to match behavior
    group.bench_function("vecdeque_cap64_with_eviction", |b| {
        b.iter(|| {
            let mut deque: VecDeque<u64> = VecDeque::with_capacity(64);
            for i in 0..iterations {
                if deque.len() >= 64 {
                    black_box(deque.pop_front());
                }
                deque.push_back(black_box(i));
            }
        })
    });

    // VecDeque unbounded (no eviction) - best case baseline
    group.bench_function("vecdeque_unbounded", |b| {
        b.iter(|| {
            let mut deque: VecDeque<u64> = VecDeque::with_capacity(iterations as usize);
            for i in 0..iterations {
                deque.push_back(black_box(i));
            }
        })
    });

    group.finish();
}

/// Compare pop performance.
fn pop_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("pop_comparison");
    let size = 1000u64;
    group.throughput(Throughput::Elements(size));

    group.bench_function("spill_ring", |b| {
        b.iter(|| {
            let ring: SpillRing<u64, 1024> = SpillRing::new();
            for i in 0..size {
                ring.push(i);
            }
            for _ in 0..size {
                black_box(ring.pop());
            }
        })
    });

    group.bench_function("vecdeque", |b| {
        b.iter(|| {
            let mut deque: VecDeque<u64> = VecDeque::with_capacity(1024);
            for i in 0..size {
                deque.push_back(i);
            }
            for _ in 0..size {
                black_box(deque.pop_front());
            }
        })
    });

    group.finish();
}

/// Compare interleaved push/pop (realistic usage).
fn interleaved_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("interleaved_comparison");
    let iterations = 10_000u64;
    group.throughput(Throughput::Elements(iterations));

    group.bench_function("spill_ring", |b| {
        b.iter(|| {
            let ring: SpillRing<u64, 64> = SpillRing::new();
            for i in 0..iterations {
                ring.push(black_box(i));
                if i % 2 == 0 {
                    black_box(ring.pop());
                }
            }
        })
    });

    group.bench_function("vecdeque", |b| {
        b.iter(|| {
            let mut deque: VecDeque<u64> = VecDeque::with_capacity(64);
            for i in 0..iterations {
                deque.push_back(black_box(i));
                if i % 2 == 0 {
                    black_box(deque.pop_front());
                }
            }
        })
    });

    group.finish();
}

/// Cache effects - small vs large capacity.
///
/// Capacity 16 (128 bytes for u64) fits in L1 cache.
/// Capacity 8192 (64KB for u64) exceeds typical L1 cache.
fn cache_effects(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_effects");
    let iterations = 50_000u64;
    group.throughput(Throughput::Elements(iterations));

    // L1-friendly (128 bytes)
    group.bench_function("cap_16_L1_friendly", |b| {
        b.iter(|| {
            let ring: SpillRing<u64, 16> = SpillRing::new();
            for i in 0..iterations {
                ring.push(black_box(i));
            }
        })
    });

    // Exceeds L1, fits L2 (~256KB typical)
    group.bench_function("cap_4096_L2", |b| {
        b.iter(|| {
            let ring: SpillRing<u64, 4096> = SpillRing::new();
            for i in 0..iterations {
                ring.push(black_box(i));
            }
        })
    });

    // Exceeds L2, hits L3 or RAM
    group.bench_function("cap_65536_L3", |b| {
        b.iter(|| {
            let ring: SpillRing<u64, 65536> = SpillRing::new();
            for i in 0..iterations {
                ring.push(black_box(i));
            }
        })
    });

    group.finish();
}

/// Eviction cost isolation - measure overhead of sink callback.
fn eviction_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("eviction_overhead");
    let iterations = 10_000u64;
    group.throughput(Throughput::Elements(iterations));

    // Small buffer = lots of evictions
    group.bench_function("cap_8_high_eviction", |b| {
        b.iter(|| {
            let ring: SpillRing<u64, 8> = SpillRing::new();
            for i in 0..iterations {
                ring.push(black_box(i));
            }
        })
    });

    // Large buffer = no evictions
    group.bench_function("cap_16384_no_eviction", |b| {
        b.iter(|| {
            let ring: SpillRing<u64, 16384> = SpillRing::new();
            for i in 0..iterations {
                ring.push(black_box(i));
            }
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    push_comparison,
    pop_comparison,
    interleaved_comparison,
    cache_effects,
    eviction_overhead,
);
criterion_main!(benches);
