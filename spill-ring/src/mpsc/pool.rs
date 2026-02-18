extern crate alloc;

use alloc::vec::Vec;
use core::marker::PhantomData;
use core::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::Arc;
use std::thread;

use crate::SpillRing;
use spout::{DropSpout, Spout};

use super::Consumer;
use super::barrier::SpinBarrier;

/// Builder for constructing a [`WorkerPool`].
///
/// Created via [`MpscRing::pool()`](super::MpscRing::pool) or
/// [`MpscRing::pool_with_sink()`](super::MpscRing::pool_with_sink).
/// Call [`spawn()`](PoolBuilder::spawn) to provide the work function and
/// start the pool.
pub struct PoolBuilder<
    T,
    const N: usize,
    S: Spout<T, Error = core::convert::Infallible> = DropSpout,
> {
    num_workers: usize,
    sink: S,
    _marker: PhantomData<T>,
}

impl<T: Send + 'static, const N: usize> PoolBuilder<T, N, DropSpout> {
    pub(crate) fn new(num_workers: usize) -> Self {
        assert!(num_workers > 0, "must have at least one worker");
        Self {
            num_workers,
            sink: DropSpout,
            _marker: PhantomData,
        }
    }
}

impl<
    T: Send + 'static,
    const N: usize,
    S: Spout<T, Error = core::convert::Infallible> + Clone + Send + 'static,
> PoolBuilder<T, N, S>
{
    pub(crate) fn with_sink(num_workers: usize, sink: S) -> Self {
        assert!(num_workers > 0, "must have at least one worker");
        Self {
            num_workers,
            sink,
            _marker: PhantomData,
        }
    }

    /// Spawn worker threads with the given work function.
    ///
    /// The work function is cloned once per thread at spawn time and
    /// monomorphized — no dynamic dispatch on the hot path. Each worker
    /// owns its own pre-warmed [`SpillRing`].
    ///
    /// All threads are spawned and cache-warmed before this returns.
    ///
    /// # Example
    ///
    /// ```
    /// use spill_ring::MpscRing;
    ///
    /// let mut pool = MpscRing::<u64, 1024>::pool(4)
    ///     .spawn(|ring, worker_id, args: &u64| {
    ///         for i in 0..*args {
    ///             ring.push(worker_id as u64 * 1000 + i);
    ///         }
    ///     });
    ///
    /// pool.run(&100);
    /// let consumer = pool.into_consumer();
    /// ```
    pub fn spawn<F, A>(self, work: F) -> WorkerPool<T, N, S, F, A>
    where
        F: Fn(&SpillRing<T, N, S>, usize, &A) + Send + Clone + 'static,
        A: Sync + 'static,
    {
        WorkerPool::start(self.num_workers, self.sink, work)
    }
}

/// A pool of persistent threads, each owning a pre-warmed [`SpillRing`].
///
/// Thread-per-core design: each thread owns its ring and executes work
/// locally. No data crosses thread boundaries on the hot path. The only
/// synchronization is barrier wake-ups between invocations.
///
/// The work function `F` is monomorphized into each thread at spawn time.
/// Per-invocation arguments `A` are passed by shared reference via atomic
/// pointer — no boxing, no cloning, no channels.
pub struct WorkerPool<T, const N: usize, S, F, A>
where
    S: Spout<T, Error = core::convert::Infallible>,
    F: Fn(&SpillRing<T, N, S>, usize, &A) + Send + Clone + 'static,
    A: Sync + 'static,
{
    num_workers: usize,
    handles: Vec<Option<thread::JoinHandle<SpillRing<T, N, S>>>>,
    args_ptr: Arc<AtomicPtr<A>>,
    shutdown: Arc<AtomicBool>,
    start_barrier: Arc<SpinBarrier>,
    done_barrier: Arc<SpinBarrier>,
    _marker: PhantomData<F>,
}

/// Shared state cloned into each worker thread.
struct WorkerCtx<A> {
    args_ptr: Arc<AtomicPtr<A>>,
    shutdown: Arc<AtomicBool>,
    ready: Arc<SpinBarrier>,
    start: Arc<SpinBarrier>,
    done: Arc<SpinBarrier>,
}

/// Worker thread entry point. Runs until shutdown is signaled, then returns the ring.
fn worker_loop<T, const N: usize, S, F, A>(
    sink: S,
    worker_id: usize,
    work: F,
    ctx: &WorkerCtx<A>,
) -> SpillRing<T, N, S>
where
    S: Spout<T, Error = core::convert::Infallible>,
    F: Fn(&SpillRing<T, N, S>, usize, &A),
{
    let ring = SpillRing::with_sink(sink);
    ctx.ready.wait();

    loop {
        ctx.start.wait();

        if ctx.shutdown.load(Ordering::Relaxed) {
            break;
        }

        // Safety: main thread sets args_ptr before triggering start barrier,
        // and args outlives run() which waits on done barrier before returning.
        let args = unsafe { &*ctx.args_ptr.load(Ordering::Acquire) };
        work(&ring, worker_id, args);

        ctx.done.wait();
    }

    ring
}

impl<T, const N: usize, S, F, A> WorkerPool<T, N, S, F, A>
where
    T: Send + 'static,
    S: Spout<T, Error = core::convert::Infallible> + Clone + Send + 'static,
    F: Fn(&SpillRing<T, N, S>, usize, &A) + Send + Clone + 'static,
    A: Sync + 'static,
{
    #[allow(clippy::needless_pass_by_value)] // sink is cloned per-worker, consumed by move
    fn start(num_workers: usize, sink: S, work: F) -> Self {
        let ready = Arc::new(SpinBarrier::new(num_workers + 1));
        let start = Arc::new(SpinBarrier::new(num_workers + 1));
        let done = Arc::new(SpinBarrier::new(num_workers + 1));
        let shutdown = Arc::new(AtomicBool::new(false));
        let args_ptr: Arc<AtomicPtr<A>> = Arc::new(AtomicPtr::new(core::ptr::null_mut()));

        let mut handles = Vec::with_capacity(num_workers);
        for worker_id in 0..num_workers {
            let ctx = WorkerCtx {
                args_ptr: Arc::clone(&args_ptr),
                shutdown: Arc::clone(&shutdown),
                ready: Arc::clone(&ready),
                start: Arc::clone(&start),
                done: Arc::clone(&done),
            };
            let handle = thread::spawn({
                let sink = sink.clone();
                let work = work.clone();
                move || worker_loop(sink, worker_id, work, &ctx)
            });
            handles.push(Some(handle));
        }

        // Wait for all threads to be spawned and warmed
        ready.wait();

        Self {
            num_workers,
            handles,
            args_ptr,
            shutdown,
            start_barrier: start,
            done_barrier: done,
            _marker: PhantomData,
        }
    }
}

impl<T, const N: usize, S, F, A> WorkerPool<T, N, S, F, A>
where
    S: Spout<T, Error = core::convert::Infallible>,
    F: Fn(&SpillRing<T, N, S>, usize, &A) + Send + Clone + 'static,
    A: Sync + 'static,
{
    /// Get the number of workers in the pool.
    #[inline]
    #[must_use]
    pub fn num_rings(&self) -> usize {
        self.num_workers
    }

    /// Run the work function on all workers with the given arguments.
    ///
    /// Each worker receives a shared reference to `args`. Blocks until
    /// all workers complete. Takes `&mut self` to prevent overlapping
    /// invocations, which would deadlock on the internal barriers.
    #[inline]
    pub fn run(&mut self, args: &A) {
        // Set args pointer before triggering start barrier
        self.args_ptr
            .store(core::ptr::from_ref(args).cast_mut(), Ordering::Release);
        self.start_barrier.wait();
        self.done_barrier.wait();
    }

    /// Convert the pool into a [`Consumer`] for draining all rings.
    ///
    /// Signals shutdown, joins all threads, and collects their rings.
    #[must_use]
    pub fn into_consumer(mut self) -> Consumer<T, N, S> {
        let rings = self.shutdown_and_join();
        let mut consumer = Consumer::new();
        for ring in rings {
            consumer.add_ring(ring);
        }
        consumer
    }

    /// Signal shutdown and join all worker threads, returning their rings.
    fn shutdown_and_join(&mut self) -> Vec<SpillRing<T, N, S>> {
        if !self.handles.iter().any(Option::is_some) {
            return Vec::new();
        }
        self.shutdown.store(true, Ordering::Relaxed);
        self.start_barrier.wait();
        self.handles
            .iter_mut()
            .filter_map(|h| h.take().map(|h| h.join().unwrap()))
            .collect()
    }
}

impl<T, const N: usize, S, F, A> Drop for WorkerPool<T, N, S, F, A>
where
    S: Spout<T, Error = core::convert::Infallible>,
    F: Fn(&SpillRing<T, N, S>, usize, &A) + Send + Clone + 'static,
    A: Sync + 'static,
{
    fn drop(&mut self) {
        self.shutdown_and_join();
    }
}
