use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::AtomicUsize;

use crate::{Flush, Spout};

/// Drops all items.
#[derive(Debug, Clone, Copy, Default)]
pub struct DropSpout;

impl<T> Spout<T> for DropSpout {
    #[inline]
    fn send(&mut self, _item: T) {}
}

/// Collects evicted items into a Vec.
#[derive(Debug, Clone, Default)]
pub struct CollectSpout<T> {
    items: Vec<T>,
}

impl<T> CollectSpout<T> {
    /// Create a new collecting spout.
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Get collected items.
    pub fn items(&self) -> &[T] {
        &self.items
    }

    /// Take collected items, leaving an empty Vec.
    pub fn take(&mut self) -> Vec<T> {
        core::mem::take(&mut self.items)
    }

    /// Consume spout and return collected items.
    pub fn into_items(self) -> Vec<T> {
        self.items
    }
}

impl<T> Spout<T> for CollectSpout<T> {
    #[inline]
    fn send(&mut self, item: T) {
        self.items.push(item);
    }

    #[inline]
    fn send_all(&mut self, items: impl Iterator<Item = T>) {
        self.items.extend(items);
    }
}

/// Calls a closure for each item.
#[derive(Debug)]
pub struct FnSpout<F>(pub F);

impl<T, F: FnMut(T)> Spout<T> for FnSpout<F> {
    #[inline]
    fn send(&mut self, item: T) {
        (self.0)(item);
    }
}

/// Calls separate closures for send and flush.
#[derive(Debug)]
pub struct FnFlushSpout<S, F> {
    send: S,
    flush: F,
}

impl<S, F> FnFlushSpout<S, F> {
    /// Create a new spout.
    pub fn new(send: S, flush: F) -> Self {
        Self { send, flush }
    }
}

impl<T, S: FnMut(T), F: Flush> Spout<T> for FnFlushSpout<S, F> {
    #[inline]
    fn send(&mut self, item: T) {
        (self.send)(item);
    }

    #[inline]
    fn flush(&mut self) {
        self.flush.flush();
    }
}

/// Create a spout from closures.
pub fn spout<T, S, F>(send: S, flush: F) -> impl Spout<T>
where
    S: FnMut(T),
    F: Flush,
{
    FnFlushSpout::new(send, flush)
}

pub struct ProducerSpout<S, F> {
    /// The inner spout (created lazily on first send)
    inner: Option<S>,
    /// Factory function to create spouts
    factory: F,
    /// This producer's ID
    producer_id: usize,
    /// Shared counter for assigning IDs
    next_id: Arc<AtomicUsize>,
}

impl<S, F: Clone> Clone for ProducerSpout<S, F> {
    fn clone(&self) -> Self {
        use core::sync::atomic::Ordering;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        Self {
            inner: None,
            factory: self.factory.clone(),
            producer_id: id,
            next_id: Arc::clone(&self.next_id),
        }
    }
}

impl<S, F> ProducerSpout<S, F>
where
    F: FnMut(usize) -> S,
{
    /// Create a new producer spout with a factory function.
    ///
    /// The factory is called with a unique producer ID (0, 1, 2, ...) for each
    /// clone, allowing creation of independent resources per producer.
    pub fn new(factory: F) -> Self {
        Self {
            inner: None,
            factory,
            producer_id: 0,
            next_id: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Get this producer's ID.
    pub fn producer_id(&self) -> usize {
        self.producer_id
    }

    /// Get a reference to the inner spout, if initialized.
    pub fn inner(&self) -> Option<&S> {
        self.inner.as_ref()
    }

    /// Get a mutable reference to the inner spout, if initialized.
    pub fn inner_mut(&mut self) -> Option<&mut S> {
        self.inner.as_mut()
    }

    /// Consume and return the inner spout, if initialized.
    pub fn into_inner(self) -> Option<S> {
        self.inner
    }

    fn ensure_inner(&mut self) {
        if self.inner.is_none() {
            self.inner = Some((self.factory)(self.producer_id));
        }
    }
}

impl<T, S, F> Spout<T> for ProducerSpout<S, F>
where
    S: Spout<T>,
    F: FnMut(usize) -> S,
{
    #[inline]
    fn send(&mut self, item: T) {
        self.ensure_inner();
        self.inner.as_mut().unwrap().send(item);
    }

    #[inline]
    fn flush(&mut self) {
        if let Some(inner) = &mut self.inner {
            inner.flush();
        }
    }
}

#[derive(Debug, Clone)]
pub struct BatchSpout<T, S> {
    pub(crate) buffer: Vec<T>,
    threshold: usize,
    sink: S,
}

impl<T, S> BatchSpout<T, S> {
    /// Create a new batch spout.
    ///
    /// Items are buffered until `threshold` items accumulate, then forwarded
    /// as a `Vec<T>` to the inner spout.
    pub fn new(threshold: usize, sink: S) -> Self {
        Self {
            buffer: Vec::with_capacity(threshold),
            threshold,
            sink,
        }
    }

    /// Get the batch threshold.
    pub fn threshold(&self) -> usize {
        self.threshold
    }

    /// Get the number of items currently buffered.
    pub fn buffered(&self) -> usize {
        self.buffer.len()
    }

    /// Get a reference to the inner spout.
    pub fn inner(&self) -> &S {
        &self.sink
    }

    /// Get a mutable reference to the inner spout.
    pub fn inner_mut(&mut self) -> &mut S {
        &mut self.sink
    }

    /// Consume and return the inner spout.
    ///
    /// Any buffered items are dropped. Call `flush()` first to forward them.
    pub fn into_inner(self) -> S {
        self.sink
    }
}

impl<T, S: Spout<Vec<T>>> Spout<T> for BatchSpout<T, S> {
    #[inline]
    fn send(&mut self, item: T) {
        self.buffer.push(item);
        if self.buffer.len() >= self.threshold {
            self.sink.send(core::mem::take(&mut self.buffer));
            self.buffer.reserve(self.threshold);
        }
    }

    #[inline]
    fn send_all(&mut self, items: impl Iterator<Item = T>) {
        for item in items {
            self.send(item);
        }
    }

    #[inline]
    fn flush(&mut self) {
        if !self.buffer.is_empty() {
            self.sink.send(core::mem::take(&mut self.buffer));
        }
        self.sink.flush();
    }
}

#[derive(Debug, Clone)]
pub struct ReduceSpout<T, R, F, S> {
    buffer: Vec<T>,
    threshold: usize,
    reduce: F,
    sink: S,
    _marker: core::marker::PhantomData<R>,
}

impl<T, R, F, S> ReduceSpout<T, R, F, S> {
    /// Create a new reduce spout.
    ///
    /// Items are buffered until `threshold` items accumulate, then `reduce`
    /// is called with the batch and the result is forwarded to the inner spout.
    pub fn new(threshold: usize, reduce: F, sink: S) -> Self {
        Self {
            buffer: Vec::with_capacity(threshold),
            threshold,
            reduce,
            sink,
            _marker: core::marker::PhantomData,
        }
    }

    /// Get the batch threshold.
    pub fn threshold(&self) -> usize {
        self.threshold
    }

    /// Get the number of items currently buffered.
    pub fn buffered(&self) -> usize {
        self.buffer.len()
    }

    /// Get a reference to the inner spout.
    pub fn inner(&self) -> &S {
        &self.sink
    }

    /// Get a mutable reference to the inner spout.
    pub fn inner_mut(&mut self) -> &mut S {
        &mut self.sink
    }

    /// Consume and return the inner spout.
    ///
    /// Any buffered items are dropped. Call `flush()` first to process them.
    pub fn into_inner(self) -> S {
        self.sink
    }
}

impl<T, R, F, S> Spout<T> for ReduceSpout<T, R, F, S>
where
    F: FnMut(Vec<T>) -> R,
    S: Spout<R>,
{
    #[inline]
    fn send(&mut self, item: T) {
        self.buffer.push(item);
        if self.buffer.len() >= self.threshold {
            let reduced = (self.reduce)(core::mem::take(&mut self.buffer));
            self.sink.send(reduced);
            self.buffer.reserve(self.threshold);
        }
    }

    #[inline]
    fn send_all(&mut self, items: impl Iterator<Item = T>) {
        for item in items {
            self.send(item);
        }
    }

    #[inline]
    fn flush(&mut self) {
        if !self.buffer.is_empty() {
            let reduced = (self.reduce)(core::mem::take(&mut self.buffer));
            self.sink.send(reduced);
        }
        self.sink.flush();
    }
}
