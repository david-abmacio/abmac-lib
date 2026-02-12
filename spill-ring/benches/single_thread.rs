//! Single-thread benchmarks — exclusive `push_mut`/`pop_mut` path (`&mut self`).
//!
//! Every benchmark in this file uses the non-atomic, exclusive-access API.
//! No cross-core traffic. This is the raw data-structure baseline.
//!
//! Rings are pre-warmed (via `SpillRing::new()`) and reused across iterations
//! via `clear()` unless noted otherwise.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use spill_ring::SpillRing;
use spout::{CollectSpout, DropSpout};
use std::collections::VecDeque;
use std::hint::black_box;

/// Push throughput with DropSpout (items discarded on eviction).
fn push_mut_drop_sink(c: &mut Criterion) {
    let mut group = c.benchmark_group("single/push_mut/drop_sink");

    for capacity in [16, 64, 256, 1024] {
        group.throughput(Throughput::Elements(10_000));
        group.bench_with_input(
            BenchmarkId::from_parameter(capacity),
            &capacity,
            |b, &cap| match cap {
                16 => {
                    let mut ring: SpillRing<u64, 16, DropSpout> = SpillRing::new();
                    b.iter(|| {
                        ring.clear();
                        for i in 0..10_000u64 {
                            ring.push_mut(black_box(i));
                        }
                    });
                }
                64 => {
                    let mut ring: SpillRing<u64, 64, DropSpout> = SpillRing::new();
                    b.iter(|| {
                        ring.clear();
                        for i in 0..10_000u64 {
                            ring.push_mut(black_box(i));
                        }
                    });
                }
                256 => {
                    let mut ring: SpillRing<u64, 256, DropSpout> = SpillRing::new();
                    b.iter(|| {
                        ring.clear();
                        for i in 0..10_000u64 {
                            ring.push_mut(black_box(i));
                        }
                    });
                }
                1024 => {
                    let mut ring: SpillRing<u64, 1024, DropSpout> = SpillRing::new();
                    b.iter(|| {
                        ring.clear();
                        for i in 0..10_000u64 {
                            ring.push_mut(black_box(i));
                        }
                    });
                }
                _ => unreachable!(),
            },
        );
    }
    group.finish();
}

/// Push throughput with CollectSpout (items collected on eviction).
///
/// CollectSpout accumulates items, so the ring + sink must be recreated
/// each iteration. Warming cost is included in measurement.
fn push_mut_collect_sink(c: &mut Criterion) {
    let mut group = c.benchmark_group("single/push_mut/collect_sink");

    for capacity in [16, 64, 256] {
        group.throughput(Throughput::Elements(10_000));
        group.bench_with_input(
            BenchmarkId::from_parameter(capacity),
            &capacity,
            |b, &cap| match cap {
                16 => b.iter(|| {
                    let mut ring: SpillRing<u64, 16, _> = SpillRing::with_sink(CollectSpout::new());
                    for i in 0..10_000u64 {
                        ring.push_mut(black_box(i));
                    }
                }),
                64 => b.iter(|| {
                    let mut ring: SpillRing<u64, 64, _> = SpillRing::with_sink(CollectSpout::new());
                    for i in 0..10_000u64 {
                        ring.push_mut(black_box(i));
                    }
                }),
                256 => b.iter(|| {
                    let mut ring: SpillRing<u64, 256, _> =
                        SpillRing::with_sink(CollectSpout::new());
                    for i in 0..10_000u64 {
                        ring.push_mut(black_box(i));
                    }
                }),
                _ => unreachable!(),
            },
        );
    }
    group.finish();
}

/// Pop throughput. Each iteration refills the ring then pops all items.
fn pop_mut_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("single/pop_mut");

    for capacity in [16, 64, 256, 1024] {
        group.throughput(Throughput::Elements(capacity as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(capacity),
            &capacity,
            |b, &cap| match cap {
                16 => {
                    let mut ring: SpillRing<u64, 16> = SpillRing::new();
                    b.iter(|| {
                        ring.clear();
                        for i in 0..16u64 {
                            ring.push_mut(i);
                        }
                        for _ in 0..16 {
                            black_box(ring.pop_mut());
                        }
                    });
                }
                64 => {
                    let mut ring: SpillRing<u64, 64> = SpillRing::new();
                    b.iter(|| {
                        ring.clear();
                        for i in 0..64u64 {
                            ring.push_mut(i);
                        }
                        for _ in 0..64 {
                            black_box(ring.pop_mut());
                        }
                    });
                }
                256 => {
                    let mut ring: SpillRing<u64, 256> = SpillRing::new();
                    b.iter(|| {
                        ring.clear();
                        for i in 0..256u64 {
                            ring.push_mut(i);
                        }
                        for _ in 0..256 {
                            black_box(ring.pop_mut());
                        }
                    });
                }
                1024 => {
                    let mut ring: SpillRing<u64, 1024> = SpillRing::new();
                    b.iter(|| {
                        ring.clear();
                        for i in 0..1024u64 {
                            ring.push_mut(i);
                        }
                        for _ in 0..1024 {
                            black_box(ring.pop_mut());
                        }
                    });
                }
                _ => unreachable!(),
            },
        );
    }
    group.finish();
}

/// Push+pop interleaved (realistic usage pattern).
fn push_pop_mut_interleaved(c: &mut Criterion) {
    let mut group = c.benchmark_group("single/push_pop_mut");

    for capacity in [16, 64, 256] {
        group.throughput(Throughput::Elements(10_000));
        group.bench_with_input(
            BenchmarkId::from_parameter(capacity),
            &capacity,
            |b, &cap| match cap {
                16 => {
                    let mut ring: SpillRing<u64, 16> = SpillRing::new();
                    b.iter(|| {
                        ring.clear();
                        for i in 0..10_000u64 {
                            ring.push_mut(black_box(i));
                            if i % 2 == 0 {
                                black_box(ring.pop_mut());
                            }
                        }
                    });
                }
                64 => {
                    let mut ring: SpillRing<u64, 64> = SpillRing::new();
                    b.iter(|| {
                        ring.clear();
                        for i in 0..10_000u64 {
                            ring.push_mut(black_box(i));
                            if i % 2 == 0 {
                                black_box(ring.pop_mut());
                            }
                        }
                    });
                }
                256 => {
                    let mut ring: SpillRing<u64, 256> = SpillRing::new();
                    b.iter(|| {
                        ring.clear();
                        for i in 0..10_000u64 {
                            ring.push_mut(black_box(i));
                            if i % 2 == 0 {
                                black_box(ring.pop_mut());
                            }
                        }
                    });
                }
                _ => unreachable!(),
            },
        );
    }
    group.finish();
}

/// Impact of item size on push throughput.
fn item_size_impact(c: &mut Criterion) {
    let mut group = c.benchmark_group("single/item_size");
    group.throughput(Throughput::Elements(10_000));

    // u64 — 8 bytes
    {
        let mut ring: SpillRing<u64, 64> = SpillRing::new();
        group.bench_function("u64_8bytes", |b| {
            b.iter(|| {
                ring.clear();
                for i in 0..10_000u64 {
                    ring.push_mut(black_box(i));
                }
            })
        });
    }

    // [u64; 4] — 32 bytes
    {
        let mut ring: SpillRing<[u64; 4], 64> = SpillRing::new();
        group.bench_function("array_32bytes", |b| {
            b.iter(|| {
                ring.clear();
                for i in 0..10_000u64 {
                    ring.push_mut(black_box([i, i, i, i]));
                }
            })
        });
    }

    // [u64; 16] — 128 bytes
    {
        let mut ring: SpillRing<[u64; 16], 64> = SpillRing::new();
        group.bench_function("array_128bytes", |b| {
            b.iter(|| {
                ring.clear();
                for i in 0..10_000u64 {
                    ring.push_mut(black_box([i; 16]));
                }
            })
        });
    }

    group.finish();
}

/// Single push latency when buffer has room.
fn push_mut_latency_not_full(c: &mut Criterion) {
    let mut group = c.benchmark_group("single/push_mut_latency");

    group.bench_function("not_full", |b| {
        let mut ring: SpillRing<u64, 256> = SpillRing::new();
        let mut i = 0u64;
        b.iter(|| {
            ring.push_mut(black_box(i));
            i = i.wrapping_add(1);
            if ring.len() > 128 {
                let _ = ring.pop_mut();
            }
        })
    });

    group.finish();
}

/// Single push latency when buffer is full (causes eviction).
fn push_mut_latency_full(c: &mut Criterion) {
    let mut group = c.benchmark_group("single/push_mut_latency");

    group.bench_function("full_evicting", |b| {
        let mut ring: SpillRing<u64, 64> = SpillRing::new();
        for i in 0..64u64 {
            ring.push_mut(i);
        }
        let mut i = 64u64;
        b.iter(|| {
            ring.push_mut(black_box(i));
            i = i.wrapping_add(1);
        })
    });

    group.finish();
}

/// Single pop latency.
fn pop_mut_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("single/pop_mut_latency");

    group.bench_function("single_pop", |b| {
        let mut ring: SpillRing<u64, 256> = SpillRing::new();
        let mut i = 0u64;
        b.iter(|| {
            if ring.len() < 128 {
                for _ in 0..64 {
                    ring.push_mut(i);
                    i = i.wrapping_add(1);
                }
            }
            black_box(ring.pop_mut())
        })
    });

    group.finish();
}

/// Peek latency (no removal).
fn peek_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("single/peek_latency");

    group.bench_function("peek", |b| {
        let mut ring: SpillRing<u64, 64> = SpillRing::new();
        for i in 0..32u64 {
            ring.push_mut(i);
        }
        b.iter(|| black_box(ring.peek().copied()))
    });

    group.bench_function("peek_back", |b| {
        let mut ring: SpillRing<u64, 64> = SpillRing::new();
        for i in 0..32u64 {
            ring.push_mut(i);
        }
        b.iter(|| black_box(ring.peek_back().copied()))
    });

    group.finish();
}

/// Drain iterator latency. Each iteration refills and drains.
fn drain_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("single/drain_latency");

    group.bench_function("drain_64_items", |b| {
        let mut ring: SpillRing<u64, 64> = SpillRing::new();
        b.iter(|| {
            for i in 0..64u64 {
                ring.push_mut(i);
            }
            let sum: u64 = ring.drain().sum();
            black_box(sum)
        })
    });

    group.finish();
}

/// Push comparison: SpillRing vs VecDeque.
fn vs_vecdeque_push(c: &mut Criterion) {
    let mut group = c.benchmark_group("single/vs_vecdeque/push");
    let iterations = 10_000u64;
    group.throughput(Throughput::Elements(iterations));

    // SpillRing — automatic eviction
    {
        let mut ring: SpillRing<u64, 64> = SpillRing::new();
        group.bench_function("spill_ring", |b| {
            b.iter(|| {
                ring.clear();
                for i in 0..iterations {
                    ring.push_mut(black_box(i));
                }
            })
        });
    }

    // VecDeque with manual eviction to match behavior
    {
        let mut deque: VecDeque<u64> = VecDeque::with_capacity(64);
        group.bench_function("vecdeque_evict", |b| {
            b.iter(|| {
                deque.clear();
                for i in 0..iterations {
                    if deque.len() >= 64 {
                        black_box(deque.pop_front());
                    }
                    deque.push_back(black_box(i));
                }
            })
        });
    }

    // VecDeque unbounded (no eviction) — best case baseline
    {
        let mut deque: VecDeque<u64> = VecDeque::with_capacity(iterations as usize);
        group.bench_function("vecdeque_unbounded", |b| {
            b.iter(|| {
                deque.clear();
                for i in 0..iterations {
                    deque.push_back(black_box(i));
                }
            })
        });
    }

    group.finish();
}

/// Pop comparison: SpillRing vs VecDeque.
fn vs_vecdeque_pop(c: &mut Criterion) {
    let mut group = c.benchmark_group("single/vs_vecdeque/pop");
    let size = 1000u64;
    group.throughput(Throughput::Elements(size));

    {
        let mut ring: SpillRing<u64, 1024> = SpillRing::new();
        group.bench_function("spill_ring", |b| {
            b.iter(|| {
                ring.clear();
                for i in 0..size {
                    ring.push_mut(i);
                }
                for _ in 0..size {
                    black_box(ring.pop_mut());
                }
            })
        });
    }

    {
        let mut deque: VecDeque<u64> = VecDeque::with_capacity(1024);
        group.bench_function("vecdeque", |b| {
            b.iter(|| {
                deque.clear();
                for i in 0..size {
                    deque.push_back(i);
                }
                for _ in 0..size {
                    black_box(deque.pop_front());
                }
            })
        });
    }

    group.finish();
}

/// Interleaved push/pop comparison.
fn vs_vecdeque_interleaved(c: &mut Criterion) {
    let mut group = c.benchmark_group("single/vs_vecdeque/interleaved");
    let iterations = 10_000u64;
    group.throughput(Throughput::Elements(iterations));

    {
        let mut ring: SpillRing<u64, 64> = SpillRing::new();
        group.bench_function("spill_ring", |b| {
            b.iter(|| {
                ring.clear();
                for i in 0..iterations {
                    ring.push_mut(black_box(i));
                    if i % 2 == 0 {
                        black_box(ring.pop_mut());
                    }
                }
            })
        });
    }

    {
        let mut deque: VecDeque<u64> = VecDeque::with_capacity(64);
        group.bench_function("vecdeque", |b| {
            b.iter(|| {
                deque.clear();
                for i in 0..iterations {
                    deque.push_back(black_box(i));
                    if i % 2 == 0 {
                        black_box(deque.pop_front());
                    }
                }
            })
        });
    }

    group.finish();
}

/// Cache effects — small vs large capacity.
fn cache_effects(c: &mut Criterion) {
    let mut group = c.benchmark_group("single/cache_effects");
    let iterations = 50_000u64;
    group.throughput(Throughput::Elements(iterations));

    // L1-friendly (128 bytes for u64)
    {
        let mut ring: SpillRing<u64, 16> = SpillRing::new();
        group.bench_function("cap_16_L1", |b| {
            b.iter(|| {
                ring.clear();
                for i in 0..iterations {
                    ring.push_mut(black_box(i));
                }
            })
        });
    }

    // Exceeds L1, fits L2
    {
        let mut ring: SpillRing<u64, 4096> = SpillRing::new();
        group.bench_function("cap_4096_L2", |b| {
            b.iter(|| {
                ring.clear();
                for i in 0..iterations {
                    ring.push_mut(black_box(i));
                }
            })
        });
    }

    // Exceeds L2, hits L3 or RAM
    {
        let mut ring: SpillRing<u64, 65536> = SpillRing::new();
        group.bench_function("cap_65536_L3", |b| {
            b.iter(|| {
                ring.clear();
                for i in 0..iterations {
                    ring.push_mut(black_box(i));
                }
            })
        });
    }

    group.finish();
}

/// Eviction cost isolation — high eviction vs no eviction.
fn eviction_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("single/eviction_overhead");
    let iterations = 10_000u64;
    group.throughput(Throughput::Elements(iterations));

    // Small buffer = lots of evictions
    {
        let mut ring: SpillRing<u64, 8> = SpillRing::new();
        group.bench_function("cap_8_high_eviction", |b| {
            b.iter(|| {
                ring.clear();
                for i in 0..iterations {
                    ring.push_mut(black_box(i));
                }
            })
        });
    }

    // Large buffer = no evictions
    {
        let mut ring: SpillRing<u64, 16384> = SpillRing::new();
        group.bench_function("cap_16384_no_eviction", |b| {
            b.iter(|| {
                ring.clear();
                for i in 0..iterations {
                    ring.push_mut(black_box(i));
                }
            })
        });
    }

    group.finish();
}

criterion_group!(
    throughput_benches,
    push_mut_drop_sink,
    push_mut_collect_sink,
    pop_mut_throughput,
    push_pop_mut_interleaved,
    item_size_impact,
);

criterion_group!(
    latency_benches,
    push_mut_latency_not_full,
    push_mut_latency_full,
    pop_mut_latency,
    peek_latency,
    drain_latency,
);

criterion_group!(
    comparison_benches,
    vs_vecdeque_push,
    vs_vecdeque_pop,
    vs_vecdeque_interleaved,
    cache_effects,
    eviction_overhead,
);

criterion_main!(throughput_benches, latency_benches, comparison_benches);
