//! A bounded worker pool on scoped threads.
//!
//! No I/O of its own; used by `cache::fetch` and `sync`. There is no
//! `Arc<Mutex<_>>`: workers pull indices from an atomic cursor over the
//! borrowed slice.

use std::num::NonZeroUsize;

/// Applies `f` to every item with at most `jobs` items in flight, and returns
/// the results in input order.
///
/// Runs inside `std::thread::scope`, so `f` and `items` may borrow from the
/// caller. Uses `min(jobs, items.len())` threads.
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
    let _ = (items, jobs, f);
    unimplemented!("W1.13")
}
