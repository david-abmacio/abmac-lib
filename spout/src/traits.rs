/// Consumes items.
pub trait Spout<T> {
    /// The error type returned by fallible operations.
    type Error;

    /// Consume an item.
    fn send(&mut self, item: T) -> Result<(), Self::Error>;

    /// Consume multiple items from an iterator.
    ///
    /// Default implementation calls `send` for each item.
    /// Implementors can override for batch optimizations.
    #[inline]
    fn send_all(&mut self, items: impl Iterator<Item = T>) -> Result<(), Self::Error> {
        for item in items {
            self.send(item)?;
        }
        Ok(())
    }

    /// Flush buffered data.
    #[inline]
    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
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

/// Wraps a closure as a [`Flush`] implementation.
#[must_use]
pub struct FlushFn<F>(pub F);

impl<F: FnMut()> Flush for FlushFn<F> {
    #[inline]
    fn flush(&mut self) {
        (self.0)()
    }
}
