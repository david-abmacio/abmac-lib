extern crate alloc;

#[cfg(test)]
mod tests;

#[cfg(feature = "std")]
use std::sync::mpsc;

/// Consumes items.
pub trait Sink<T> {
    /// Consume an item.
    fn send(&mut self, item: T);

    /// Consume multiple items from an iterator.
    ///
    /// Default implementation calls `send` for each item.
    /// Implementors can override for batch optimizations.
    #[inline]
    fn send_all(&mut self, items: impl Iterator<Item = T>) {
        for item in items {
            self.send(item);
        }
    }

    /// Flush buffered data.
    #[inline]
    fn flush(&mut self) {}
}

/// Drops all items.
#[derive(Debug, Clone, Copy, Default)]
pub struct DropSink;

impl<T> Sink<T> for DropSink {
    #[inline]
    fn send(&mut self, _item: T) {}
}

/// Collects evicted items into a Vec.
#[derive(Debug, Clone, Default)]
pub struct CollectSink<T> {
    items: Vec<T>,
}

impl<T> CollectSink<T> {
    /// Create a new collecting sink.
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

    /// Consume sink and return collected items.
    pub fn into_items(self) -> Vec<T> {
        self.items
    }
}

impl<T> Sink<T> for CollectSink<T> {
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
pub struct FnSink<F>(pub F);

impl<T, F: FnMut(T)> Sink<T> for FnSink<F> {
    #[inline]
    fn send(&mut self, item: T) {
        (self.0)(item);
    }
}

/// Calls separate closures for send and flush.
#[derive(Debug)]
pub struct FnFlushSink<S, F> {
    send: S,
    flush: F,
}

impl<S, F> FnFlushSink<S, F> {
    /// Create a new sink.
    pub fn new(send: S, flush: F) -> Self {
        Self { send, flush }
    }
}

impl<T, S: FnMut(T), F: Flush> Sink<T> for FnFlushSink<S, F> {
    #[inline]
    fn send(&mut self, item: T) {
        (self.send)(item);
    }

    #[inline]
    fn flush(&mut self) {
        self.flush.flush();
    }
}

/// Flush behavior.
pub trait Flush {
    /// Perform flush.
    fn flush(&mut self);
}

impl Flush for () {
    #[inline]
    fn flush(&mut self) {}
}

impl<F: FnMut()> Flush for F {
    #[inline]
    fn flush(&mut self) {
        self()
    }
}

/// Create a sink from closures.
pub fn sink<T, S, F>(send: S, flush: F) -> impl Sink<T>
where
    S: FnMut(T),
    F: Flush,
{
    FnFlushSink::new(send, flush)
}

#[cfg(feature = "std")]
#[derive(Debug, Clone)]
pub struct ChannelSink<T> {
    sender: mpsc::Sender<T>,
}

#[cfg(feature = "std")]
impl<T> ChannelSink<T> {
    /// Create a new channel sink from a sender.
    pub fn new(sender: mpsc::Sender<T>) -> Self {
        Self { sender }
    }

    /// Get a reference to the underlying sender.
    pub fn sender(&self) -> &mpsc::Sender<T> {
        &self.sender
    }

    /// Consume the sink and return the sender.
    pub fn into_sender(self) -> mpsc::Sender<T> {
        self.sender
    }
}

#[cfg(feature = "std")]
impl<T> Sink<T> for ChannelSink<T> {
    #[inline]
    fn send(&mut self, item: T) {
        // Ignore send errors - receiver may have been dropped
        let _ = self.sender.send(item);
    }
}

/// Thread-safe sink wrapper using `Arc<Mutex<S>>`.
///
/// Allows multiple producers to share a single sink with mutex synchronization.
/// Useful for MPSC patterns where all items should go to one collector.
#[cfg(feature = "std")]
impl<T, S: Sink<T>> Sink<T> for std::sync::Arc<std::sync::Mutex<S>> {
    #[inline]
    fn send(&mut self, item: T) {
        self.lock().unwrap().send(item);
    }

    #[inline]
    fn flush(&mut self) {
        self.lock().unwrap().flush();
    }
}

pub struct ProducerSink<S, F> {
    /// The inner sink (created lazily on first send)
    inner: Option<S>,
    /// Factory function to create sinks
    factory: F,
    /// This producer's ID
    producer_id: usize,
    /// Shared counter for assigning IDs
    next_id: alloc::sync::Arc<core::sync::atomic::AtomicUsize>,
}

impl<S, F: Clone> Clone for ProducerSink<S, F> {
    fn clone(&self) -> Self {
        use core::sync::atomic::Ordering;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        Self {
            inner: None,
            factory: self.factory.clone(),
            producer_id: id,
            next_id: alloc::sync::Arc::clone(&self.next_id),
        }
    }
}

impl<S, F> ProducerSink<S, F>
where
    F: FnMut(usize) -> S,
{
    /// Create a new producer sink with a factory function.
    ///
    /// The factory is called with a unique producer ID (0, 1, 2, ...) for each
    /// clone, allowing creation of independent resources per producer.
    pub fn new(factory: F) -> Self {
        Self {
            inner: None,
            factory,
            producer_id: 0,
            next_id: alloc::sync::Arc::new(core::sync::atomic::AtomicUsize::new(0)),
        }
    }

    /// Get this producer's ID.
    pub fn producer_id(&self) -> usize {
        self.producer_id
    }

    /// Get a reference to the inner sink, if initialized.
    pub fn inner(&self) -> Option<&S> {
        self.inner.as_ref()
    }

    /// Get a mutable reference to the inner sink, if initialized.
    pub fn inner_mut(&mut self) -> Option<&mut S> {
        self.inner.as_mut()
    }

    /// Consume and return the inner sink, if initialized.
    pub fn into_inner(self) -> Option<S> {
        self.inner
    }

    fn ensure_inner(&mut self) {
        if self.inner.is_none() {
            self.inner = Some((self.factory)(self.producer_id));
        }
    }
}

impl<T, S, F> Sink<T> for ProducerSink<S, F>
where
    S: Sink<T>,
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
pub struct BatchSink<T, S> {
    buffer: Vec<T>,
    threshold: usize,
    sink: S,
}

impl<T, S> BatchSink<T, S> {
    /// Create a new batch sink.
    ///
    /// Items are buffered until `threshold` items accumulate, then forwarded
    /// as a `Vec<T>` to the inner sink.
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

    /// Get a reference to the inner sink.
    pub fn inner(&self) -> &S {
        &self.sink
    }

    /// Get a mutable reference to the inner sink.
    pub fn inner_mut(&mut self) -> &mut S {
        &mut self.sink
    }

    /// Consume and return the inner sink.
    ///
    /// Any buffered items are dropped. Call `flush()` first to forward them.
    pub fn into_inner(self) -> S {
        self.sink
    }
}

impl<T, S: Sink<Vec<T>>> Sink<T> for BatchSink<T, S> {
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
pub struct ReduceSink<T, R, F, S> {
    buffer: Vec<T>,
    threshold: usize,
    reduce: F,
    sink: S,
    _marker: core::marker::PhantomData<R>,
}

impl<T, R, F, S> ReduceSink<T, R, F, S> {
    /// Create a new reduce sink.
    ///
    /// Items are buffered until `threshold` items accumulate, then `reduce`
    /// is called with the batch and the result is forwarded to the inner sink.
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

    /// Get a reference to the inner sink.
    pub fn inner(&self) -> &S {
        &self.sink
    }

    /// Get a mutable reference to the inner sink.
    pub fn inner_mut(&mut self) -> &mut S {
        &mut self.sink
    }

    /// Consume and return the inner sink.
    ///
    /// Any buffered items are dropped. Call `flush()` first to process them.
    pub fn into_inner(self) -> S {
        self.sink
    }
}

impl<T, R, F, S> Sink<T> for ReduceSink<T, R, F, S>
where
    F: FnMut(Vec<T>) -> R,
    S: Sink<R>,
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
