extern crate std;

use crate::MpscRing;
use spout::{CollectSpout, ProducerSpout};

#[test]
fn basic_mpsc() {
    let (producers, mut consumer) = MpscRing::<u64, 8>::with_consumer(2);

    // Producer 0
    producers[0].push(1);
    producers[0].push(2);

    // Producer 1
    producers[1].push(10);
    producers[1].push(20);

    assert_eq!(producers[0].len(), 2);
    assert_eq!(producers[1].len(), 2);

    // Collect producers into consumer, then drain
    consumer.collect(producers);
    let mut spout = CollectSpout::new();
    consumer.drain(&mut spout);

    let mut items = spout.into_items();
    items.sort_unstable();
    assert_eq!(items, std::vec![1, 2, 10, 20]);
}

#[test]
fn producer_overflow_to_spout() {
    let spout = ProducerSpout::new(|_id| CollectSpout::<u64>::new());
    let producers = MpscRing::<u64, 4, _>::with_spout(1, spout);
    let producer = producers.into_iter().next().unwrap();

    // Overflow — push 10 items into ring of size 4
    for i in 0..10 {
        producer.push(i);
    }

    // Ring should have last 4 items, first 6 evicted to spout
    assert_eq!(producer.len(), 4);

    let ring = producer.into_ring();
    let evicted = ring.spout().inner().unwrap().items();
    assert_eq!(evicted, &[0, 1, 2, 3, 4, 5]);
    assert_eq!(ring.pop(), Some(6));
    assert_eq!(ring.pop(), Some(7));
    assert_eq!(ring.pop(), Some(8));
    assert_eq!(ring.pop(), Some(9));
}

#[test]
fn single_producer() {
    let (producers, mut consumer) = MpscRing::<u64, 16>::with_consumer(1);

    let producer = producers.into_iter().next().unwrap();
    for i in 0..10 {
        producer.push(i);
    }

    assert_eq!(producer.len(), 10);

    consumer.collect(std::vec![producer]);
    let mut spout = CollectSpout::new();
    consumer.drain(&mut spout);

    assert_eq!(spout.into_items(), std::vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
}

#[cfg(feature = "std")]
mod worker_pool_tests {
    use crate::MpscRing;
    use spout::CollectSpout;

    #[test]
    fn basic_worker_pool() {
        let mut pool = MpscRing::<u64, 64>::pool(2).spawn(|ring, _id, count: &u64| {
            for i in 0..*count {
                ring.push(i);
            }
        });

        pool.run(&50);

        let mut consumer = pool.into_consumer();
        let mut spout = CollectSpout::new();
        consumer.drain(&mut spout);

        let items = spout.into_items();
        assert_eq!(items.len(), 100); // 2 workers * 50 each
    }

    #[test]
    fn worker_pool_overflow() {
        // Small ring to force overflow
        let mut pool = MpscRing::<u64, 8>::pool(1).spawn(|ring, _id, count: &u64| {
            for i in 0..*count {
                ring.push(i);
            }
        });

        pool.run(&100);

        let mut consumer = pool.into_consumer();
        let mut spout = CollectSpout::new();
        consumer.drain(&mut spout);

        // Only last 8 items should remain in ring (92..100)
        let items = spout.into_items();
        assert_eq!(items, vec![92, 93, 94, 95, 96, 97, 98, 99]);
    }

    #[test]
    fn worker_pool_empty() {
        let pool = MpscRing::<u64, 64>::pool(4).spawn(|_ring, _id, _args: &()| {});
        let consumer = pool.into_consumer();
        assert!(consumer.is_empty());
        assert_eq!(consumer.num_producers(), 4);
    }

    #[test]
    fn worker_pool_multiple_run_calls() {
        let mut pool = MpscRing::<u64, 128>::pool(2).spawn(|ring, _id, count: &u64| {
            for i in 0..*count {
                ring.push(i);
            }
        });

        pool.run(&10);
        pool.run(&10);

        let mut consumer = pool.into_consumer();
        let mut spout = CollectSpout::new();
        consumer.drain(&mut spout);

        // Should have 40 items total (2 rings x 20 items each)
        let items = spout.into_items();
        assert_eq!(items.len(), 40);
    }

    #[test]
    fn worker_pool_worker_ids() {
        let mut pool = MpscRing::<u64, 64>::pool(4).spawn(|ring, id, _args: &()| {
            ring.push(id as u64);
        });

        pool.run(&());

        let mut consumer = pool.into_consumer();
        let mut spout = CollectSpout::new();
        consumer.drain(&mut spout);

        let mut ids = spout.into_items();
        ids.sort_unstable();
        assert_eq!(ids, vec![0, 1, 2, 3]);
    }

    #[test]
    fn worker_pool_different_args_per_run() {
        let mut pool = MpscRing::<u64, 128>::pool(1).spawn(|ring, _id, val: &u64| {
            ring.push(*val);
        });

        pool.run(&42);
        pool.run(&99);

        let mut consumer = pool.into_consumer();
        let mut spout = CollectSpout::new();
        consumer.drain(&mut spout);

        let items = spout.into_items();
        assert_eq!(items, vec![42, 99]);
    }

    #[test]
    fn worker_pool_with_spout() {
        use spout::ProducerSpout;

        let spout = ProducerSpout::new(|_id| CollectSpout::<u64>::new());

        let mut pool =
            MpscRing::<u64, 4, _>::pool_with_spout(2, spout).spawn(|ring, _id, count: &u64| {
                for i in 0..*count {
                    ring.push(i);
                }
            });

        // Push 10 items per worker into ring of size 4 — forces overflow to spout
        pool.run(&10);

        let mut consumer = pool.into_consumer();
        let mut drain_spout = CollectSpout::new();
        consumer.drain(&mut drain_spout);

        // Each worker has a ring of size 4, pushed 10 items → last 4 remain per ring
        let items = drain_spout.into_items();
        assert_eq!(items.len(), 8);
        // Each worker's ring should contain [6, 7, 8, 9] (last 4 of 0..10)
        // Items drain in producer order: worker0's 4, then worker1's 4
        assert_eq!(&items[..4], &[6, 7, 8, 9]);
        assert_eq!(&items[4..], &[6, 7, 8, 9]);
    }

    #[test]
    fn worker_pool_drop_without_consume() {
        let mut pool = MpscRing::<u64, 64>::pool(4).spawn(|ring, _id, count: &u64| {
            for i in 0..*count {
                ring.push(i);
            }
        });

        pool.run(&100);
        drop(pool); // Should not panic or hang
    }

    #[test]
    fn worker_panic_does_not_deadlock() {
        use std::panic;

        let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            let mut pool = MpscRing::<u64, 64>::pool(2).spawn(|_ring, id, _args: &()| {
                if id == 0 {
                    panic!("intentional test panic");
                }
            });
            pool.run(&());
        }));

        // run() should propagate the panic, not deadlock.
        assert!(result.is_err());
    }

    #[test]
    fn try_run_returns_error_on_panic() {
        let mut pool = MpscRing::<u64, 64>::pool(2).spawn(|_ring, id, _args: &()| {
            if id == 0 {
                panic!("intentional test panic");
            }
        });

        // First call triggers the panic.
        // Give the panicking worker a moment to actually panic.
        std::thread::sleep(std::time::Duration::from_millis(10));
        let result = pool.try_run(&());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.worker_id, 0);
    }

    #[test]
    fn drop_after_panic_does_not_abort() {
        // If Drop double-panics, the process aborts. This test verifies
        // that dropping a pool with a panicked worker is graceful.
        let pool = MpscRing::<u64, 64>::pool(2).spawn(|_ring, id, _args: &()| {
            if id == 1 {
                panic!("intentional test panic");
            }
        });
        // Just drop the pool — should not abort.
        drop(pool);
    }

    #[test]
    fn multiple_run_cycles_accumulate_items() {
        // Verify items from multiple run() calls are all collected.
        let mut pool = MpscRing::<u64, 128>::pool(2).spawn(|ring, id, round: &u64| {
            ring.push(id as u64 * 100 + *round);
        });

        pool.run(&1);
        pool.run(&2);
        pool.run(&3);

        let mut consumer = pool.into_consumer();
        let mut spout = CollectSpout::new();
        consumer.drain(&mut spout);

        let mut items = spout.into_items();
        items.sort_unstable();
        // Worker 0: 1, 2, 3. Worker 1: 101, 102, 103.
        assert_eq!(items, vec![1, 2, 3, 101, 102, 103]);
    }

    // --- Feature 1: Streaming Collection API ---

    #[test]
    fn dispatch_join_collect() {
        // dispatch + join + collect should produce same results as run + into_consumer
        let mut pool = MpscRing::<u64, 256>::pool(4).spawn(|ring, id, count: &u64| {
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
    fn collect_between_runs() {
        // collect() picks up batches from previous run()
        let mut pool = MpscRing::<u64, 256>::pool(2).spawn(|ring, _id, count: &u64| {
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
    fn collect_multiple_rounds_then_into_consumer() {
        // collect() over 3 rounds, then into_consumer gets nothing extra
        let mut pool = MpscRing::<u64, 256>::pool(2).spawn(|ring, _id, _args: &()| {
            ring.push(1);
        });

        let mut sink = CollectSpout::new();
        for _ in 0..3 {
            pool.dispatch(&());
            pool.join().unwrap();
            pool.collect(&mut sink).unwrap();
        }
        assert_eq!(sink.items().len(), 6); // 2 workers * 3 rounds

        // into_consumer should get the final rings (empty since we collected)
        let consumer = pool.into_consumer();
        assert!(consumer.is_empty());
    }

    #[test]
    fn collect_empty_slots_skipped() {
        // collect() when no batches published yet returns 0
        let mut pool = MpscRing::<u64, 256>::pool(2).spawn(|ring, _id, _args: &()| {
            ring.push(1);
        });

        // No dispatch yet — slots are empty
        let mut sink = CollectSpout::new();
        let count = pool.collect(&mut sink).unwrap();
        assert_eq!(count, 0);
        assert!(sink.items().is_empty());
    }

    #[test]
    fn collect_propagates_spout_error() {
        use std::sync::mpsc;

        let mut pool = MpscRing::<u64, 256>::pool(1).spawn(|ring, _id, _args: &()| {
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
}

// --- Missing coverage tests ---

#[test]
fn with_consumer_and_collect() {
    let (producers, mut consumer) = MpscRing::<u64, 16>::with_consumer(3);

    producers[0].push(1);
    producers[0].push(2);
    producers[1].push(10);
    producers[2].push(100);

    consumer.collect(producers);

    assert_eq!(consumer.num_producers(), 3);
    assert_eq!(consumer.len(), 4);
    assert!(!consumer.is_empty());

    let mut spout = CollectSpout::new();
    consumer.drain(&mut spout);

    let mut items = spout.into_items();
    items.sort_unstable();
    assert_eq!(items, std::vec![1, 2, 10, 100]);

    assert!(consumer.is_empty());
    assert_eq!(consumer.len(), 0);
}

#[cfg(feature = "std")]
#[test]
fn multi_threaded_producers() {
    use std::thread;

    let (producers, mut consumer) = MpscRing::<u64, 64>::with_consumer(4);

    let finished: std::vec::Vec<_> = thread::scope(|s| {
        producers
            .into_iter()
            .enumerate()
            .map(|(id, producer)| {
                s.spawn(move || {
                    for i in 0..100 {
                        producer.push(id as u64 * 1000 + i);
                    }
                    producer
                })
            })
            .collect::<std::vec::Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect()
    });

    consumer.collect(finished);

    let mut spout = CollectSpout::new();
    consumer.drain(&mut spout);

    let items = spout.into_items();
    // 4 threads x 100 items, ring capacity 64 so last 64 per thread survive
    assert_eq!(items.len(), 256);

    // Verify each thread's items are the last 64 (36..100)
    for thread_id in 0..4u64 {
        let thread_items: std::vec::Vec<u64> = items
            .iter()
            .filter(|&&x| x / 1000 == thread_id)
            .copied()
            .collect();
        assert_eq!(thread_items.len(), 64);
        let expected: std::vec::Vec<u64> = (36..100).map(|i| thread_id * 1000 + i).collect();
        assert_eq!(thread_items, expected);
    }
}
