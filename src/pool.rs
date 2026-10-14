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

#[cfg(test)]
mod tests {
    use super::*;
    use std::assert_matches;
    use std::sync::{Condvar, Mutex};
    use std::time::Duration;

    fn nz(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).unwrap()
    }

    #[test]
    fn keeps_input_order() {
        let items: Vec<usize> = (0..100).collect();
        let out = run_bounded(&items, nz(8), |n| n * 2);
        assert_eq!(out, (0..100).map(|n| n * 2).collect::<Vec<_>>());
    }

    #[test]
    fn concurrency_never_exceeds_jobs_and_reaches_it() {
        let jobs = 4;
        let items: Vec<usize> = (0..32).collect();
        let active = AtomicUsize::new(0);
        let high = AtomicUsize::new(0);
        // The first `jobs` items rendezvous, forcing real overlap. A timeout
        // fails the test instead of hanging if fewer workers start.
        let arrived = Mutex::new(0usize);
        let cv = Condvar::new();
        let timed_out = AtomicBool::new(false);
        let out = run_bounded(&items, nz(jobs), |&n| {
            let now = active.fetch_add(1, Ordering::SeqCst) + 1;
            high.fetch_max(now, Ordering::SeqCst);
            if n < jobs {
                let mut n_arrived = arrived.lock().unwrap();
                *n_arrived += 1;
                cv.notify_all();
                let (_guard, res) = cv
                    .wait_timeout_while(n_arrived, Duration::from_secs(10), |c| *c < jobs)
                    .unwrap();
                if res.timed_out() {
                    timed_out.store(true, Ordering::SeqCst);
                }
            }
            active.fetch_sub(1, Ordering::SeqCst);
            n
        });
        assert!(
            !timed_out.load(Ordering::SeqCst),
            "fewer than {jobs} workers"
        );
        assert_eq!(out, items);
        assert_eq!(high.load(Ordering::SeqCst), jobs);
    }

    #[test]
    fn single_job_runs_serially() {
        let active = AtomicUsize::new(0);
        let high = AtomicUsize::new(0);
        let out = run_bounded(&[1, 2, 3, 4], nz(1), |&n| {
            let now = active.fetch_add(1, Ordering::SeqCst) + 1;
            high.fetch_max(now, Ordering::SeqCst);
            active.fetch_sub(1, Ordering::SeqCst);
            n
        });
        assert_eq!(out, [1, 2, 3, 4]);
        assert_eq!(high.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn empty_input_returns_empty() {
        let out: Vec<u8> = run_bounded(&[] as &[u8], nz(4), |&n| n);
        assert_eq!(out, Vec::<u8>::new());
    }

    #[test]
    fn jobs_larger_than_input_works() {
        let out = run_bounded(&["a", "b"], nz(64), |s| s.to_uppercase());
        assert_eq!(out, ["A", "B"]);
    }

    #[test]
    fn worker_panic_propagates() {
        let result = std::panic::catch_unwind(|| {
            run_bounded(&[1, 2, 3], nz(2), |&n| {
                assert!(n != 2, "boom");
                n
            })
        });
        let payload = result.expect_err("panic must propagate");
        let msg = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str));
        assert_matches!(msg, Some(m) if m.contains("boom"));
    }

    #[test]
    fn panic_stops_remaining_items() {
        let ran = AtomicUsize::new(0);
        let items: Vec<usize> = (0..10_000).collect();
        let result = std::panic::catch_unwind(|| {
            run_bounded(&items, nz(1), |&n| {
                ran.fetch_add(1, Ordering::SeqCst);
                assert!(n != 0, "boom");
            })
        });
        assert_matches!(result, Err(_));
        assert_eq!(ran.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn huge_jobs_is_clamped() {
        let items: Vec<usize> = (0..1_000).collect();
        let out = run_bounded(&items, nz(usize::MAX), |&n| n);
        assert_eq!(out, items);
    }
}
