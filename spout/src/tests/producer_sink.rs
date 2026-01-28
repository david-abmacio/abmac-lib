extern crate std;

use crate::{CollectSink, ProducerSink, Sink};

#[test]
fn producer_sink_assigns_unique_ids() {
    let sink = ProducerSink::new(|_id| CollectSink::<i32>::new());

    let sink0 = sink.clone();
    let sink1 = sink.clone();
    let sink2 = sink.clone();

    assert_eq!(sink0.producer_id(), 0);
    assert_eq!(sink1.producer_id(), 1);
    assert_eq!(sink2.producer_id(), 2);
}

#[test]
fn producer_sink_creates_independent_sinks() {
    let sink = ProducerSink::new(|_id| CollectSink::<i32>::new());

    let mut sink0 = sink.clone();
    let mut sink1 = sink.clone();

    sink0.send(1);
    sink0.send(2);
    sink1.send(10);

    // Each has its own collected items
    assert_eq!(sink0.inner().unwrap().items(), &[1, 2]);
    assert_eq!(sink1.inner().unwrap().items(), &[10]);
}

#[test]
fn producer_sink_lazy_init() {
    let sink = ProducerSink::new(|_id| CollectSink::<i32>::new());

    let sink0 = sink.clone();

    // Inner not initialized until first send
    assert!(sink0.inner().is_none());
}

#[test]
fn producer_sink_flush_delegates() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static FLUSH_COUNT: AtomicUsize = AtomicUsize::new(0);

    struct FlushCounter;
    impl Sink<i32> for FlushCounter {
        fn send(&mut self, _item: i32) {}
        fn flush(&mut self) {
            FLUSH_COUNT.fetch_add(1, Ordering::SeqCst);
        }
    }

    FLUSH_COUNT.store(0, Ordering::SeqCst);

    let sink = ProducerSink::new(|_id| FlushCounter);
    let mut sink0 = sink.clone();

    // Flush before init is a no-op
    sink0.flush();
    assert_eq!(FLUSH_COUNT.load(Ordering::SeqCst), 0);

    // Send initializes, then flush delegates
    sink0.send(1);
    sink0.flush();
    assert_eq!(FLUSH_COUNT.load(Ordering::SeqCst), 1);
}

#[test]
fn producer_sink_into_inner() {
    let sink = ProducerSink::new(|_id| CollectSink::<i32>::new());
    let mut sink0 = sink.clone();

    sink0.send(42);

    let inner = sink0.into_inner().unwrap();
    assert_eq!(inner.items(), &[42]);
}

#[test]
fn producer_sink_with_mpsc_ring() {
    use crate::MpscRing;

    let sink = ProducerSink::new(|_id| CollectSink::<u64>::new());

    let producers = MpscRing::<u64, 4, _>::with_sink(3, sink);

    // Push enough to cause evictions
    for (i, producer) in producers.into_iter().enumerate() {
        for j in 0..10u64 {
            producer.push(i as u64 * 100 + j);
        }
        // Producer drops here, remaining items flush to sink
    }
}
