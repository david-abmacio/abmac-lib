use crate::MpscRing;
use spout::CollectSpout;

// --- Streaming Guards ---

#[test]
fn streaming_fan_in_basic() {
    use crate::UnorderedCollector;

    let mut pool = MpscRing::<u64, 256>::pool(4).spawn(|ring, id, count: &u64| {
        for i in 0..*count {
            ring.push(id as u64 * 10000 + i);
        }
    });

    let mut stream = pool.streaming_fan_in(UnorderedCollector::new(CollectSpout::new()));

    for _ in 0..3 {
        stream.run(&50);
        stream.flush().unwrap();
    }

    let items = stream.into_collector().into_inner().into_items();
    // 4 workers * 50 items * 3 rounds = 600
    assert_eq!(items.len(), 600);
}

#[test]
fn streaming_fan_in_sequenced() {
    use crate::SequencedCollector;

    let mut pool = MpscRing::<u64, 256>::pool(4).spawn(|ring, id, round: &u64| {
        ring.push(*round * 100 + id as u64);
    });

    let mut stream = pool.streaming_fan_in(SequencedCollector::from_spout(CollectSpout::new()));

    for round in 0..5u64 {
        stream.dispatch(&round);
        stream.join().unwrap();
        stream.flush().unwrap();
    }

    let items = stream.into_collector().into_inner_spout().into_items();
    assert_eq!(items.len(), 20);

    // Verify ordering: round values must be monotonically non-decreasing.
    let mut last_round = 0u64;
    for &item in &items {
        let item_round = item / 100;
        assert!(
            item_round >= last_round,
            "ordering violation: item {item} (round {item_round}) after round {last_round}"
        );
        last_round = item_round;
    }
}

#[test]
fn streaming_fan_in_reuse_pool() {
    use crate::UnorderedCollector;

    let mut pool = MpscRing::<u64, 256>::pool(2).spawn(|ring, _id, _args: &()| {
        ring.push(1);
    });

    // Use streaming guard, then drop it.
    {
        let mut stream = pool.streaming_fan_in(UnorderedCollector::new(CollectSpout::new()));
        stream.run(&());
        stream.flush().unwrap();
        let items = stream.into_collector().into_inner().into_items();
        assert_eq!(items.len(), 2);
    }

    // Pool regains &mut self — can dispatch again.
    pool.dispatch(&());
    pool.join().unwrap();

    let mut sink = CollectSpout::new();
    pool.collect(&mut sink).unwrap();
    assert_eq!(sink.items().len(), 2);
}

#[test]
fn streaming_mergers_basic() {
    use crate::UnorderedCollector;

    let mut pool = MpscRing::<u64, 256>::pool(8).spawn(|ring, id, _args: &()| {
        ring.push(id as u64);
    });

    let mut stream = pool.streaming_mergers(2, |_| UnorderedCollector::new(CollectSpout::new()));

    stream.run(&());

    std::thread::scope(|s| {
        let handles: std::vec::Vec<_> = stream
            .mergers()
            .iter_mut()
            .map(|m| s.spawn(|| m.flush().unwrap()))
            .collect();
        for h in handles {
            h.join().unwrap();
        }
    });

    let mut all_items = std::vec::Vec::new();
    for merger in stream.mergers().iter() {
        all_items.extend_from_slice(merger.collector().inner().items());
    }
    all_items.sort();
    assert_eq!(all_items, std::vec![0, 1, 2, 3, 4, 5, 6, 7]);
}

#[test]
fn streaming_mergers_into_mergers() {
    use crate::UnorderedCollector;

    let mut pool = MpscRing::<u64, 256>::pool(4).spawn(|ring, id, _args: &()| {
        ring.push(id as u64);
    });

    let mut stream = pool.streaming_mergers(2, |_| UnorderedCollector::new(CollectSpout::new()));

    stream.run(&());
    for m in stream.mergers().iter_mut() {
        m.flush().unwrap();
    }

    let mergers = stream.into_mergers();
    assert_eq!(mergers.len(), 2);

    let mut all_items = std::vec::Vec::new();
    for merger in &mergers {
        all_items.extend_from_slice(merger.collector().inner().items());
    }
    all_items.sort();
    assert_eq!(all_items, std::vec![0, 1, 2, 3]);
}

// --- Convenience method tests ---

#[test]
fn streaming_fan_in_collect_convenience() {
    let mut pool = MpscRing::<u64, 256>::pool(4).spawn(|ring, id, count: &u64| {
        for i in 0..*count {
            ring.push(id as u64 * 10000 + i);
        }
    });

    let mut stream = pool.streaming_fan_in_collect();

    for _ in 0..3 {
        stream.run(&50);
        stream.flush().unwrap();
    }

    let items = stream.into_collector().into_inner().into_items();
    assert_eq!(items.len(), 600);
}

#[test]
fn streaming_fan_in_collect_sequenced_convenience() {
    let mut pool = MpscRing::<u64, 256>::pool(4).spawn(|ring, id, round: &u64| {
        ring.push(*round * 100 + id as u64);
    });

    let mut stream = pool.streaming_fan_in_collect_sequenced();

    for round in 0..5u64 {
        stream.dispatch(&round);
        stream.join().unwrap();
        stream.flush().unwrap();
    }

    let items = stream.into_collector().into_inner_spout().into_items();
    assert_eq!(items.len(), 20);

    let mut last_round = 0u64;
    for &item in &items {
        let item_round = item / 100;
        assert!(
            item_round >= last_round,
            "ordering violation: item {item} (round {item_round}) after round {last_round}"
        );
        last_round = item_round;
    }
}

#[test]
fn streaming_mergers_collect_convenience() {
    let mut pool = MpscRing::<u64, 256>::pool(8).spawn(|ring, id, _args: &()| {
        ring.push(id as u64);
    });

    let mut stream = pool.streaming_mergers_collect(2);

    stream.run(&());

    std::thread::scope(|s| {
        let handles: std::vec::Vec<_> = stream
            .mergers()
            .iter_mut()
            .map(|m| s.spawn(|| m.flush().unwrap()))
            .collect();
        for h in handles {
            h.join().unwrap();
        }
    });

    let mut all_items = std::vec::Vec::new();
    for merger in stream.mergers().iter() {
        all_items.extend_from_slice(merger.collector().inner().items());
    }
    all_items.sort();
    assert_eq!(all_items, std::vec![0, 1, 2, 3, 4, 5, 6, 7]);
}
