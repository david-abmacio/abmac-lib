//! Latency benchmarks - per-operation timing.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use spill_ring::SpillRing;

/// Benchmark single push latency when buffer has room.
fn push_latency_not_full(c: &mut Criterion) {
    let mut group = c.benchmark_group("push_latency");

    group.bench_function("not_full", |b| {
        let ring: SpillRing<u64, 256> = SpillRing::new();
        let mut i = 0u64;
        b.iter(|| {
            ring.push(black_box(i));
            i = i.wrapping_add(1);
            // Pop to keep room
            if ring.len() > 128 {
                let _ = ring.pop();
            }
        })
    });

    group.finish();
}

/// Benchmark single push latency when buffer is full (causes eviction).
fn push_latency_full(c: &mut Criterion) {
    let mut group = c.benchmark_group("push_latency");

    group.bench_function("full_evicting", |b| {
        let ring: SpillRing<u64, 64> = SpillRing::new();
        // Fill it up
        for i in 0..64u64 {
            ring.push(i);
        }
        let mut i = 64u64;
        b.iter(|| {
            ring.push(black_box(i));
            i = i.wrapping_add(1);
        })
    });

    group.finish();
}

/// Benchmark single pop latency.
fn pop_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("pop_latency");

    group.bench_function("single_pop", |b| {
        let ring: SpillRing<u64, 256> = SpillRing::new();
        let mut i = 0u64;
        b.iter(|| {
            // Keep buffer half-full
            if ring.len() < 128 {
                for _ in 0..64 {
                    ring.push(i);
                    i = i.wrapping_add(1);
                }
            }
            black_box(ring.pop())
        })
    });

    group.finish();
}

/// Benchmark peek latency (no removal).
fn peek_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("peek_latency");

    group.bench_function("peek", |b| {
        let ring: SpillRing<u64, 64> = SpillRing::new();
        for i in 0..32u64 {
            ring.push(i);
        }
        b.iter(|| black_box(ring.peek()))
    });

    group.bench_function("peek_back", |b| {
        let ring: SpillRing<u64, 64> = SpillRing::new();
        for i in 0..32u64 {
            ring.push(i);
        }
        b.iter(|| black_box(ring.peek_back()))
    });

    group.finish();
}

/// Benchmark drain iterator.
fn drain_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("drain_latency");

    group.bench_function("drain_64_items", |b| {
        b.iter(|| {
            let mut ring: SpillRing<u64, 64> = SpillRing::new();
            for i in 0..64u64 {
                ring.push(i);
            }
            let sum: u64 = ring.drain().sum();
            black_box(sum)
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    push_latency_not_full,
    push_latency_full,
    pop_latency,
    peek_latency,
    drain_latency,
);
criterion_main!(benches);
