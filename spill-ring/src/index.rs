//! Index and cell abstractions for ring buffer internals.

use core::cell::UnsafeCell;

// SpoutCell

/// Interior mutable cell for spout.
#[repr(transparent)]
pub(crate) struct SpoutCell<S>(UnsafeCell<S>);

impl<S> SpoutCell<S> {
    #[inline]
    pub(crate) const fn new(sink: S) -> Self {
        Self(UnsafeCell::new(sink))
    }

    /// # Safety
    /// Caller must ensure exclusive access.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub(crate) unsafe fn get_mut_unchecked(&self) -> &mut S {
        unsafe { &mut *self.0.get() }
    }

    /// # Safety
    /// Caller must ensure no `&mut S` alias exists for the duration of
    /// the returned reference.
    #[inline]
    pub(crate) unsafe fn get_ref(&self) -> &S {
        unsafe { &*self.0.get() }
    }

    #[inline]
    pub(crate) fn get_mut(&mut self) -> &mut S {
        self.0.get_mut()
    }
}

// SAFETY: SpoutCell wraps UnsafeCell<S>, which is !Sync by default. Send is
// safe when S: Send because transferring ownership to another thread does not
// create concurrent access — SpoutCell is only accessed from one thread at a time.
unsafe impl<S: Send> Send for SpoutCell<S> {}

// CellIndex

mod non_atomic {
    use core::cell::Cell;

    /// Non-atomic index for single-context use.
    #[repr(transparent)]
    pub(crate) struct CellIndex(Cell<usize>);

    impl CellIndex {
        #[inline]
        pub const fn new(val: usize) -> Self {
            Self(Cell::new(val))
        }

        #[inline]
        pub fn load(&self) -> usize {
            self.0.get()
        }

        #[inline]
        pub fn store(&self, val: usize) {
            self.0.set(val);
        }

        /// Load without Cell overhead (exclusive access).
        #[inline]
        pub fn load_mut(&mut self) -> usize {
            *self.0.get_mut()
        }

        /// Store without Cell overhead (exclusive access).
        #[inline]
        pub fn store_mut(&mut self, val: usize) {
            *self.0.get_mut() = val;
        }
    }
}

pub(crate) use non_atomic::CellIndex;
