//! Sink traits and implementations.

extern crate alloc;

use alloc::vec::Vec;

/// Consumes items.
pub trait Sink<T> {
    /// Consume an item.
    fn send(&mut self, item: T);

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
