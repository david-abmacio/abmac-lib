//! SPSC (Single-Producer, Single-Consumer) concurrent benchmarks.

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use spill_ring::SpillRing;
use std::sync::Arc;
use std::thread;

/// Benchmark SPSC throughput with varying buffer sizes.
fn spsc_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("spsc_throughput");

    for capacity in [16, 64, 256] {
        let iterations = 100_000u64;
        group.throughput(Throughput::Elements(iterations));
        group.bench_with_input(
            BenchmarkId::from_parameter(capacity),
            &capacity,
            |b, &cap| match cap {
                16 => b.iter(|| spsc_run::<16>(iterations)),
                64 => b.iter(|| spsc_run::<64>(iterations)),
                256 => b.iter(|| spsc_run::<256>(iterations)),
                _ => unreachable!(),
            },
        );
    }
    group.finish();
}

fn spsc_run<const N: usize>(iterations: u64) -> u64 {
    let ring = Arc::new(SpillRing::<u64, N>::new());

    let producer_ring = Arc::clone(&ring);
    let producer = thread::spawn(move || {
        for i in 0..iterations {
            producer_ring.push(black_box(i));
        }
    });

    let consumer_ring = Arc::clone(&ring);
    let consumer = thread::spawn(move || {
        let mut count = 0u64;
        let mut spins = 0;
        loop {
            if let Some(v) = consumer_ring.pop() {
                black_box(v);
                count += 1;
                spins = 0;
            } else {
                spins += 1;
                if spins > 10_000 {
                    break;
                }
                std::hint::spin_loop();
            }
        }
        count
    });

    producer.join().unwrap();
    consumer.join().unwrap()
}

/// Benchmark SPSC with producer faster than consumer (backpressure).
fn spsc_producer_faster(c: &mut Criterion) {
    let mut group = c.benchmark_group("spsc_producer_faster");

    let iterations = 50_000u64;
    group.throughput(Throughput::Elements(iterations));

    group.bench_function("cap_64", |b| {
        b.iter(|| {
            let ring = Arc::new(SpillRing::<u64, 64>::new());

            let producer_ring = Arc::clone(&ring);
            let producer = thread::spawn(move || {
                for i in 0..iterations {
                    producer_ring.push(black_box(i));
                    // Producer is fast - no delay
                }
            });

            let consumer_ring = Arc::clone(&ring);
            let consumer = thread::spawn(move || {
                let mut count = 0u64;
                let mut spins = 0;
                loop {
                    if let Some(v) = consumer_ring.pop() {
                        black_box(v);
                        count += 1;
                        spins = 0;
                        // Consumer is slow - small delay
                        for _ in 0..10 {
                            std::hint::spin_loop();
                        }
                    } else {
                        spins += 1;
                        if spins > 10_000 {
                            break;
                        }
                    }
                }
                count
            });

            producer.join().unwrap();
            consumer.join().unwrap()
        })
    });

    group.finish();
}

/// Benchmark SPSC with consumer faster than producer (no backpressure).
fn spsc_consumer_faster(c: &mut Criterion) {
    let mut group = c.benchmark_group("spsc_consumer_faster");

    let iterations = 50_000u64;
    group.throughput(Throughput::Elements(iterations));

    group.bench_function("cap_64", |b| {
        b.iter(|| {
            let ring = Arc::new(SpillRing::<u64, 64>::new());

            let producer_ring = Arc::clone(&ring);
            let producer = thread::spawn(move || {
                for i in 0..iterations {
                    producer_ring.push(black_box(i));
                    // Producer is slow
                    for _ in 0..10 {
                        std::hint::spin_loop();
                    }
                }
            });

            let consumer_ring = Arc::clone(&ring);
            let consumer = thread::spawn(move || {
                let mut count = 0u64;
                let mut spins = 0;
                loop {
                    if let Some(v) = consumer_ring.pop() {
                        black_box(v);
                        count += 1;
                        spins = 0;
                        // Consumer is fast - no delay
                    } else {
                        spins += 1;
                        if spins > 50_000 {
                            break;
                        }
                        std::hint::spin_loop();
                    }
                }
                count
            });

            producer.join().unwrap();
            consumer.join().unwrap()
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    spsc_throughput,
    spsc_producer_faster,
    spsc_consumer_faster,
);
criterion_main!(benches);
