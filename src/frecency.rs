//! `frecency.json`: repo and query frecency on disk.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::core::frecency::{Frecency, FrecencyData};
use crate::core::query_frecency::QueryFrecency;
use crate::domain::{Slug, StoredTime};
use crate::state_file::{StateError, StateFile};

pub use crate::state_file::Problem;

/// File name inside the cache directory.
pub const FRECENCY_FILE: &str = "frecency.json";

/// The only version this build reads or writes.
pub const FRECENCY_VERSION: u32 = 1;

/// Entries whose latest access is older than this are dropped on write.
pub const RETENTION: Duration = Duration::from_hours(365 * 24);

/// Repo and query frecency: the contents of `frecency.json` minus its
/// version.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct History {
    /// Repo frecency, keyed by slug.
    #[serde(default)]
    pub frecency: Frecency,
    /// Query-to-repo frecency.
    #[serde(default)]
    pub query_frecency: QueryFrecency,
}

impl History {
    /// Drops every entry whose latest access is more than [`RETENTION`]
    /// before `now`, and every query left with no entries.
    pub fn prune(&mut self, now: StoredTime) {
        let fresh = |_: &Slug, d: &FrecencyData| d.last_access.age_at(now) <= RETENTION;
        self.frecency.retain(fresh);
        self.query_frecency.retain(fresh);
    }
}

/// `frecency.json`.
pub static FRECENCY: StateFile = StateFile {
    name: FRECENCY_FILE,
    version: FRECENCY_VERSION,
    noun: "frecency",
    fresh: "a new history",
};

/// Saving `frecency.json` failed.
pub type FrecencyError = StateError;

/// `{dir}/frecency.json`.
#[must_use]
pub fn frecency_file(dir: &Path) -> PathBuf {
    FRECENCY.path(dir)
}

/// The history in `dir`, and the problem to warn about, if any; any problem
/// comes with an empty history. An unparsable file is moved aside under the
/// lock, only once a re-read confirms it is still unparsable.
#[must_use]
pub fn load(dir: &Path) -> (History, Option<Problem>) {
    FRECENCY.load(dir)
}

/// Records into `{dir}/frecency.json`: under the lock, reloads the file,
/// applies `f`, prunes entries older than [`RETENTION`] before `now`, and
/// writes the result atomically, so concurrent `work` processes never lose
/// each other's accesses.
///
/// A file of unknown version or one that cannot be read is never
/// overwritten; nothing is written and the problem is returned.
///
/// # Errors
///
/// [`FrecencyError`] when encoding or writing fails.
pub fn record(
    dir: &Path,
    now: StoredTime,
    f: impl FnOnce(&mut History),
) -> Result<Option<Problem>, FrecencyError> {
    let updated = FRECENCY.update(dir, |history: &mut History| {
        f(history);
        history.prune(now);
    });
    match updated {
        Ok(((), problem)) => Ok(problem),
        Err(StateError::ReadOnly(problem)) => Ok(Some(problem)),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::domain::QueryKey;

    const DAY: i64 = 86_400;
    const NOW: i64 = 1_000 * DAY;

    fn t(secs: i64) -> StoredTime {
        StoredTime::from_unix_seconds(secs).unwrap()
    }

    fn slug(name: &str) -> Slug {
        Slug::try_from(format!("github.com/acme/{name}")).unwrap()
    }

    fn count(h: &History, name: &str) -> Option<u32> {
        h.frecency.get(&slug(name)).map(|d| d.access_count)
    }

    #[test]
    fn a_write_drops_entries_older_than_a_year() {
        let dir = tempfile::tempdir().unwrap();
        let key = QueryKey::new("x").unwrap();
        record(dir.path(), t(NOW), |h| {
            h.frecency.record(&slug("old"), t(NOW - 366 * DAY));
            h.frecency.record(&slug("kept"), t(NOW - 365 * DAY));
            h.query_frecency
                .record(&key, &slug("old"), t(NOW - 366 * DAY));
        })
        .unwrap();
        let (h, _) = load(dir.path());
        assert_eq!(count(&h, "old"), None);
        assert_eq!(count(&h, "kept"), Some(1));
        assert!(h.query_frecency.get(&key).is_none());
    }

    #[test]
    fn concurrent_recorders_lose_nothing() {
        const ROUNDS: u32 = 40;
        let dir = tempfile::tempdir().unwrap();
        std::thread::scope(|s| {
            for name in ["a", "b"] {
                let path = dir.path();
                s.spawn(move || {
                    for _ in 0..ROUNDS {
                        record(path, t(NOW), |h| h.frecency.record(&slug(name), t(NOW))).unwrap();
                    }
                });
            }
        });
        let (h, _) = load(dir.path());
        assert_eq!(count(&h, "a"), Some(ROUNDS));
        assert_eq!(count(&h, "b"), Some(ROUNDS));
    }
}
