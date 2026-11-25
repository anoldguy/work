//! Repo and query frecency scoring and `frecency.json` persistence.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::domain::{QueryKey, Slug, StoredTime};
use crate::state_file::{StateError, StateFile};

pub use crate::state_file::Problem;

/// File name inside the cache directory.
pub(crate) const FRECENCY_FILE: &str = "frecency.json";

/// The only version this build reads or writes.
pub(crate) const FRECENCY_VERSION: u32 = 1;

/// Entries whose latest access is older than this are dropped on write.
pub(crate) const RETENTION: Duration = Duration::from_hours(365 * 24);

const SECS_PER_DAY: u64 = 86_400;

/// How often and how recently something was accessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrecencyData {
    /// Number of recorded accesses.
    pub access_count: u32,
    /// When the latest access was recorded.
    pub last_access: StoredTime,
}

/// Weight of an access that is `age_days` old: 1.0 up to 7 days, 0.5 up to
/// 30, 0.2 up to 90, and 0.05 after that.
///
/// ```
/// use work::frecency::recency_multiplier;
///
/// assert_eq!(recency_multiplier(0), 1.0);
/// assert_eq!(recency_multiplier(7), 1.0);
/// assert_eq!(recency_multiplier(8), 0.5);
/// assert_eq!(recency_multiplier(90), 0.2);
/// assert_eq!(recency_multiplier(365), 0.05);
/// ```
#[must_use]
pub const fn recency_multiplier(age_days: u64) -> f64 {
    match age_days {
        0..=7 => 1.0,
        8..=30 => 0.5,
        31..=90 => 0.2,
        _ => 0.05,
    }
}

fn data_score(d: &FrecencyData, now: StoredTime) -> f64 {
    // Rounded up, so "7 days" means at most 7 full days. A `last_access` in
    // the future (clock skew) has age 0 (`age_at` clamps) and scores 1.0.
    let age_days = d.last_access.age_at(now).as_secs().div_ceil(SECS_PER_DAY);
    f64::from(d.access_count) * recency_multiplier(age_days)
}

/// Frecency per repo, keyed by slug. Serialised as a plain JSON object
/// (`{"github.com/acme/foo": {"access_count": 3, "last_access": "..."}}`).
///
/// Keys compare exactly (case-sensitive).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Frecency(BTreeMap<Slug, FrecencyData>);

impl Frecency {
    /// Counts one access to `s` at `now`.
    pub fn record(&mut self, s: &Slug, now: StoredTime) {
        if let Some(entry) = self.0.get_mut(s) {
            entry.access_count = entry.access_count.saturating_add(1);
            entry.last_access = now;
        } else {
            self.0.insert(
                s.clone(),
                FrecencyData {
                    access_count: 1,
                    last_access: now,
                },
            );
        }
    }

    /// `access_count * recency_multiplier(age in days)`, or 0 for a slug with
    /// no history.
    #[must_use]
    pub fn score(&self, s: &Slug, now: StoredTime) -> f64 {
        self.0.get(s).map_or(0.0, |d| data_score(d, now))
    }

    /// The highest-scoring entry (with its score), and the best score among
    /// all *other* entries (0 when there is none). Ties break by slug
    /// ascending.
    #[must_use]
    pub fn top_two(&self, now: StoredTime) -> (Option<(&Slug, &FrecencyData, f64)>, f64) {
        // BTreeMap iterates in slug order, so a strict `>` keeps the
        // smallest slug among equal scores.
        let mut top: Option<(&Slug, &FrecencyData, f64)> = None;
        let mut runner_up = 0.0_f64;
        for (slug, data) in &self.0 {
            let score = data_score(data, now);
            match top {
                Some((_, _, best)) if score <= best => runner_up = runner_up.max(score),
                Some((_, _, best)) => {
                    runner_up = runner_up.max(best);
                    top = Some((slug, data, score));
                }
                None => top = Some((slug, data, score)),
            }
        }
        (top, runner_up)
    }

    /// Keeps only the entries for which `keep` returns true.
    pub fn retain(&mut self, mut keep: impl FnMut(&Slug, &FrecencyData) -> bool) {
        self.0.retain(|slug, data| keep(slug, data));
    }

    /// The raw entry for `s`, if any.
    #[cfg(test)]
    #[must_use]
    pub fn get(&self, s: &Slug) -> Option<&FrecencyData> {
        self.0.get(s)
    }

    /// True when nothing has been recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Per-query repo frecency: `{"foo": {"github.com/acme/foobar": {...}}}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct QueryFrecency(BTreeMap<QueryKey, Frecency>);

impl QueryFrecency {
    /// Counts one resolution of `k` to `s` at `now`.
    pub fn record(&mut self, k: &QueryKey, s: &Slug, now: StoredTime) {
        if let Some(history) = self.0.get_mut(k) {
            history.record(s, now);
        } else {
            let mut history = Frecency::default();
            history.record(s, now);
            self.0.insert(k.clone(), history);
        }
    }

    /// Keeps only the entries for which `keep` returns true, then drops
    /// queries left with no entries.
    pub fn retain(&mut self, mut keep: impl FnMut(&Slug, &FrecencyData) -> bool) {
        for history in self.0.values_mut() {
            history.retain(&mut keep);
        }
        self.0.retain(|_, history| !history.is_empty());
    }

    /// The history recorded for `k`, if any.
    #[must_use]
    pub fn get(&self, k: &QueryKey) -> Option<&Frecency> {
        self.0.get(k)
    }
}

/// Repo and query frecency: the contents of `frecency.json` minus its
/// version.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct History {
    /// Repo frecency, keyed by slug.
    #[serde(default)]
    pub(crate) frecency: Frecency,
    /// Query-to-repo frecency.
    #[serde(default)]
    pub(crate) query_frecency: QueryFrecency,
}

impl History {
    /// Drops every entry whose latest access is more than [`RETENTION`]
    /// before `now`, and every query left with no entries.
    pub(crate) fn prune(&mut self, now: StoredTime) {
        let fresh = |_: &Slug, d: &FrecencyData| d.last_access.age_at(now) <= RETENTION;
        self.frecency.retain(fresh);
        self.query_frecency.retain(fresh);
    }
}

/// `frecency.json`.
pub(crate) static FRECENCY: StateFile = StateFile {
    name: FRECENCY_FILE,
    version: FRECENCY_VERSION,
    noun: "frecency",
    fresh: "a new history",
};

/// The history in `dir`, and the problem to warn about, if any; any problem
/// comes with an empty history. An unparsable file is moved aside under the
/// lock, only once a re-read confirms it is still unparsable.
#[must_use]
pub(crate) fn load(dir: &Path) -> (History, Option<Problem>) {
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
/// [`StateError`] when encoding or writing fails.
pub(crate) fn record(
    dir: &Path,
    now: StoredTime,
    f: impl FnOnce(&mut History),
) -> Result<Option<Problem>, StateError> {
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
    fn score_uses_whole_days_at_boundaries() {
        let s = slug("b");
        let mut f = Frecency::default();
        f.record(&s, t(0));
        f.record(&s, t(0));
        assert_eq!(f.score(&s, t(7 * DAY)), 2.0);
        assert_eq!(f.score(&s, t(7 * DAY + 1)), 1.0);
        assert_eq!(f.score(&s, t(8 * DAY)), 1.0);
        assert_eq!(f.score(&s, t(30 * DAY)), 1.0);
        assert_eq!(f.score(&s, t(31 * DAY)), 0.4);
        assert_eq!(f.score(&s, t(90 * DAY)), 0.4);
        assert_eq!(f.score(&s, t(91 * DAY)), 0.1);
        assert_eq!(f.score(&slug("none"), t(0)), 0.0);
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
