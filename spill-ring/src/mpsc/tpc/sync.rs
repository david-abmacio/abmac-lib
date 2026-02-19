//! Adaptive spin-then-yield synchronization utilities.

use std::thread;

/// Compute spin limit based on thread count vs available cores.
///
/// When threads <= available cores, spin longer (no contention for CPU).
/// When oversubscribed, yield immediately to avoid starvation.
/// Each `spin_loop` hint is ~5ns on x86, so 256 spins ~ 1.3us.
pub(crate) fn compute_spin_limit(num_threads: usize) -> u32 {
    let cores = thread::available_parallelism()
        .map(std::num::NonZero::get)
        .unwrap_or(1);
    if num_threads > cores {
        0
    } else {
        #[allow(clippy::cast_possible_truncation)]
        ((cores - num_threads + 1) as u32 * 32).min(256)
    }
}

/// Adaptive spin-then-yield: spin for `spin_limit` iterations, then yield.
///
/// Returns when `condition()` returns `true`.
#[inline]
pub(crate) fn spin_wait(spin_limit: u32, condition: impl Fn() -> bool) {
    let mut spins = 0u32;
    while !condition() {
        if spins < spin_limit {
            core::hint::spin_loop();
            spins += 1;
        } else {
            thread::yield_now();
            spins = 0;
        }
    }
}
