use core::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

/// Spin-barrier for low-latency thread synchronization.
///
/// Unlike `std::sync::Barrier` (Mutex + Condvar), this spins on atomics
/// with no OS syscalls. Eliminates the ~58µs overhead at 8 threads that
/// dominates short work units.
pub(crate) struct SpinBarrier {
    count: AtomicUsize,
    generation: AtomicUsize,
    num_threads: usize,
    /// Spin iterations before yielding. Scaled to hardware at construction:
    /// when threads <= available cores, spin longer (no contention for CPU);
    /// when oversubscribed, yield immediately to avoid starvation.
    spin_limit: u32,
}

impl SpinBarrier {
    pub(crate) fn new(num_threads: usize) -> Self {
        let cores = thread::available_parallelism()
            .map(std::num::NonZero::get)
            .unwrap_or(1);
        // Oversubscribed: yield immediately. Otherwise spin proportional
        // to headroom — more spare cores means less risk of starving.
        let spin_limit = if num_threads > cores {
            0
        } else {
            // 32 spins per spare core, capped at 256. Each PAUSE is ~5ns
            // on x86, so 256 spins ≈ 1.3µs — well under a scheduler tick.
            #[allow(clippy::cast_possible_truncation)]
            ((cores - num_threads + 1) as u32 * 32).min(256)
        };

        Self {
            count: AtomicUsize::new(0),
            generation: AtomicUsize::new(0),
            num_threads,
            spin_limit,
        }
    }

    pub(crate) fn wait(&self) {
        let epoch = self.generation.load(Ordering::Relaxed);

        if self.count.fetch_add(1, Ordering::AcqRel) + 1 == self.num_threads {
            // Last thread to arrive — reset count and advance generation.
            self.count.store(0, Ordering::Relaxed);
            self.generation
                .store(epoch.wrapping_add(1), Ordering::Release);
        } else {
            let mut spins = 0u32;
            while self.generation.load(Ordering::Acquire) == epoch {
                if spins < self.spin_limit {
                    std::hint::spin_loop();
                    spins += 1;
                } else {
                    thread::yield_now();
                }
            }
        }
    }
}
