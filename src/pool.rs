//! A bounded worker pool on scoped threads.
//!
//! No I/O of its own; used by `cache::fetch` and `sync`. There is no
//! `Arc<Mutex<_>>`: workers pull indices from an atomic cursor over the
//! borrowed slice.

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;

/// Upper bound on worker threads, however large `jobs` is.
const MAX_WORKERS: usize = 256;

/// Applies `f` to every item with at most `jobs` items in flight, and returns
/// the results in input order.
///
/// Runs inside `std::thread::scope`, so `f` and `items` may borrow from the
/// caller. Uses `min(jobs, items.len(), 256)` threads; if the OS refuses a
/// thread, the pool carries on with the workers it got (or runs on the
/// calling thread when it got none).
///
/// After a worker panics, the other workers stop taking new items.
///
/// # Panics
///
/// Re-raises a panic from `f` once every worker has stopped.
#[must_use]
pub fn run_bounded<T: Sync, R: Send>(
    items: &[T],
    jobs: NonZeroUsize,
    f: impl Fn(&T) -> R + Sync,
) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let failed = AtomicBool::new(false);
    let workers = jobs.get().min(items.len()).min(MAX_WORKERS);
    let (f, next, failed) = (&f, &next, &failed);
    let work = move || {
        let mut out = Vec::new();
        while !failed.load(Ordering::Relaxed) {
            let i = next.fetch_add(1, Ordering::Relaxed);
            let Some(item) = items.get(i) else { break };
            let guard = PanicFlag(failed);
            out.push((i, f(item)));
            std::mem::forget(guard);
        }
        out
    };
    let mut tagged: Vec<(usize, R)> = thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map_while(|_| thread::Builder::new().spawn_scoped(scope, work).ok())
            .collect();
        let mut all = if handles.is_empty() {
            work()
        } else {
            Vec::new()
        };
        for h in handles {
            all.extend(h.join().unwrap_or_else(|p| std::panic::resume_unwind(p)));
        }
        all
    });
    tagged.sort_unstable_by_key(|(i, _)| *i);
    tagged.into_iter().map(|(_, r)| r).collect()
}

/// Raises the flag when dropped during unwinding; `forget` disarms it.
struct PanicFlag<'a>(&'a AtomicBool);

impl Drop for PanicFlag<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}
