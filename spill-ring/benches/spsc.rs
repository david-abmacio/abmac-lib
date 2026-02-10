//! SPSC (Single-Producer, Single-Consumer) concurrent benchmarks.
//!
//! All benchmarks use `push(&self)`/`pop(&self)` via `Arc` — the atomic
//! Lamport path. Persistent producer + consumer threads are kept alive across
//! criterion iterations to avoid measuring `thread::spawn` overhead.
//!
//! Protocol: a shared `AtomicU64` round counter is bumped by the main thread.
//! Both worker threads spin until they see the counter advance, run their
//! workload, then signal done. The main thread waits for both done flags
//! before bumping the counter again.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use spill_ring::SpscRing;
use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};

// ---------------------------------------------------------------------------
// Shared state between main thread and workers
// ---------------------------------------------------------------------------

struct SharedState<const N: usize> {
    ring: SpscRing<u64, N>,
    /// Monotonically increasing round number. Workers spin until this advances.
    round: AtomicU64,
    /// How many items to push/pop each round.
    count: AtomicU64,
    /// Set by producer when done pushing this round.
    producer_done: AtomicBool,
    /// Set by consumer when done consuming this round.
    consumer_done: AtomicBool,
    /// Set to true to shut down both threads.
    shutdown: AtomicBool,
}

struct SpscHarness<const N: usize> {
    state: Arc<SharedState<N>>,
    current_round: u64,
    producer: Option<JoinHandle<()>>,
    consumer: Option<JoinHandle<()>>,
}

impl<const N: usize> SpscHarness<N> {
    fn new(producer_delay: u32, consumer_delay: u32) -> Self {
        let state = Arc::new(SharedState {
            ring: SpscRing::new(),
            round: AtomicU64::new(0),
            count: AtomicU64::new(0),
            producer_done: AtomicBool::new(false),
            consumer_done: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
        });

        let p_state = Arc::clone(&state);
        let producer = thread::spawn(move || {
            let mut last_round = 0u64;
            loop {
                // Spin until round advances
                let round = loop {
                    if p_state.shutdown.load(Ordering::Relaxed) {
                        return;
                    }
                    let r = p_state.round.load(Ordering::Acquire);
                    if r > last_round {
                        break r;
                    }
                    std::hint::spin_loop();
                };
                last_round = round;

                let count = p_state.count.load(Ordering::Relaxed);
                for i in 0..count {
                    p_state.ring.push(black_box(i));
                    for _ in 0..producer_delay {
                        std::hint::spin_loop();
                    }
                }
                p_state.producer_done.store(true, Ordering::Release);
            }
        });

        let c_state = Arc::clone(&state);
        let consumer = thread::spawn(move || {
            let mut last_round = 0u64;
            loop {
                // Spin until round advances
                let round = loop {
                    if c_state.shutdown.load(Ordering::Relaxed) {
                        return;
                    }
                    let r = c_state.round.load(Ordering::Acquire);
                    if r > last_round {
                        break r;
                    }
                    std::hint::spin_loop();
                };
                last_round = round;

                let count = c_state.count.load(Ordering::Relaxed);
                let mut consumed = 0u64;
                loop {
                    if let Some(v) = c_state.ring.pop() {
                        black_box(v);
                        consumed += 1;
                        for _ in 0..consumer_delay {
                            std::hint::spin_loop();
                        }
                        if consumed >= count {
                            break;
                        }
                    } else if c_state.producer_done.load(Ordering::Acquire) {
                        // Producer finished — drain whatever remains and move on.
                        // Some items may have been evicted (spilled).
                        while c_state.ring.pop().is_some() {}
                        break;
                    } else {
                        std::hint::spin_loop();
                    }
                }
                c_state.consumer_done.store(true, Ordering::Release);
            }
        });

        Self {
            state,
            current_round: 0,
            producer: Some(producer),
            consumer: Some(consumer),
        }
    }

    /// Run one iteration: push+pop `count` items through the ring.
    fn run(&mut self, count: u64) {
        // Reset done flags
        self.state.producer_done.store(false, Ordering::Relaxed);
        self.state.consumer_done.store(false, Ordering::Relaxed);
        self.state.count.store(count, Ordering::Relaxed);

        // Bump round — releases the count store above
        self.current_round += 1;
        self.state
            .round
            .store(self.current_round, Ordering::Release);

        // Wait for both threads to finish
        while !self.state.producer_done.load(Ordering::Acquire) {
            std::hint::spin_loop();
        }
        while !self.state.consumer_done.load(Ordering::Acquire) {
            std::hint::spin_loop();
        }
    }
}

impl<const N: usize> Drop for SpscHarness<N> {
    fn drop(&mut self) {
        self.state.shutdown.store(true, Ordering::Release);
        // Bump round to unblock threads that may be waiting
        self.state
            .round
            .store(self.current_round + 1, Ordering::Release);
        if let Some(h) = self.producer.take() {
            let _ = h.join();
        }
        if let Some(h) = self.consumer.take() {
            let _ = h.join();
        }
    }
}

// ---------------------------------------------------------------------------
// Benchmarks
// ---------------------------------------------------------------------------

/// SPSC throughput with persistent threads — balanced producer/consumer.
fn spsc_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("spsc/push_pop/throughput");

    let iterations = 100_000u64;

    for capacity in [16, 64, 256] {
        group.throughput(Throughput::Elements(iterations));
        group.bench_with_input(
            BenchmarkId::from_parameter(capacity),
            &capacity,
            |b, &cap| match cap {
                16 => {
                    let mut harness = SpscHarness::<16>::new(0, 0);
                    b.iter(|| harness.run(iterations));
                }
                64 => {
                    let mut harness = SpscHarness::<64>::new(0, 0);
                    b.iter(|| harness.run(iterations));
                }
                256 => {
                    let mut harness = SpscHarness::<256>::new(0, 0);
                    b.iter(|| harness.run(iterations));
                }
                _ => unreachable!(),
            },
        );
    }
    group.finish();
}

/// Producer outpaces consumer — high eviction pressure.
fn spsc_producer_faster(c: &mut Criterion) {
    let mut group = c.benchmark_group("spsc/push_pop/producer_faster");

    let iterations = 50_000u64;
    group.throughput(Throughput::Elements(iterations));

    group.bench_function("cap_64", |b| {
        let mut harness = SpscHarness::<64>::new(0, 10);
        b.iter(|| harness.run(iterations));
    });

    group.finish();
}

/// Consumer outpaces producer — consumer spins on empty ring.
fn spsc_consumer_faster(c: &mut Criterion) {
    let mut group = c.benchmark_group("spsc/push_pop/consumer_faster");

    let iterations = 50_000u64;
    group.throughput(Throughput::Elements(iterations));

    group.bench_function("cap_64", |b| {
        let mut harness = SpscHarness::<64>::new(10, 0);
        b.iter(|| harness.run(iterations));
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
