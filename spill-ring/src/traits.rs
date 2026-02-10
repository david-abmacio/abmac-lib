/// Common ring buffer properties.
/// Provides size and capacity information shared by both producers and consumers.
pub trait RingInfo {
    /// Returns the number of items currently in the ring.
    fn len(&self) -> usize;

    /// Returns the total capacity of the ring.
    fn capacity(&self) -> usize;

    /// Returns `true` if the ring contains no items.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns `true` if the ring has no available capacity.
    fn is_full(&self) -> bool {
        self.len() >= self.capacity()
    }
}

/// Producer side of a ring buffer.
///
/// Methods for pushing items into the ring. Implementations
/// may have different overflow behavior (block, fail, or spill to sink).
///
/// See [`SpillRing`](crate::SpillRing) for the primary implementation.
pub trait RingProducer<T>: RingInfo {
    /// Attempts to push an item into the ring.
    ///
    /// # Errors:
    /// Returns `Ok(())` if successful, or `Err(item)` if the ring is full.
    fn try_push(&mut self, item: T) -> Result<(), T>;
}

/// Consumer side of a ring buffer.
///
/// Methods for reading and removing items from the ring.
///
/// See [`SpillRing`](crate::SpillRing) for the primary implementation.
pub trait RingConsumer<T>: RingInfo {
    /// Attempts to pop the oldest item from the ring.
    ///
    /// Returns `Some(item)` if the ring was non-empty, or `None` if empty.
    #[must_use]
    fn try_pop(&mut self) -> Option<T>;

    /// Returns a reference to the oldest item without removing it.
    ///
    /// Returns `None` if the ring is empty.
    #[must_use]
    fn peek(&mut self) -> Option<&T>;
}

/// Combined producer and consumer trait.
///
/// Automatically implemented for any type that implements both
///
/// [`RingProducer`] and [`RingConsumer`].
pub trait RingTrait<T>: RingProducer<T> + RingConsumer<T> {}

impl<T, R: RingProducer<T> + RingConsumer<T>> RingTrait<T> for R {}
