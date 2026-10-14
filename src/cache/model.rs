//! The cache data and its staleness rule (pure).

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::core::frecency::Frecency;
use crate::core::query_frecency::QueryFrecency;
use crate::domain::{Repo, StoredTime};

/// Version written to new caches. A file with any other version is treated
/// as corrupt (renamed and rebuilt).
pub const CACHE_VERSION: u32 = 1;

/// A cache older than this is refreshed by `Freshness::AutoUpdate`.
pub const STALE_AFTER: Duration = Duration::from_hours(24);

/// How far in the future `updated_at` may be before the cache counts as stale.
pub const CLOCK_SKEW: Duration = Duration::from_mins(5);

/// File name inside the cache directory.
pub const CACHE_FILE: &str = "cache.json";

/// Everything in `cache.json`. Unknown fields are ignored; missing frecency
/// maps default to empty.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cache {
    /// Always [`CACHE_VERSION`] when written by this build.
    pub version: u32,
    /// When the repo list was last refreshed.
    pub updated_at: StoredTime,
    /// Every cached repo, de-duplicated by slug.
    pub repos: Vec<Repo>,
    /// Repo frecency.
    #[serde(default)]
    pub frecency: Frecency,
    /// Query-to-repo frecency.
    #[serde(default)]
    pub query_frecency: QueryFrecency,
}

impl Cache {
    /// A fresh cache with no history.
    #[must_use]
    pub fn new(repos: Vec<Repo>, now: StoredTime) -> Self {
        Self {
            version: CACHE_VERSION,
            updated_at: now,
            repos,
            frecency: Frecency::default(),
            query_frecency: QueryFrecency::default(),
        }
    }

    /// True when `updated_at` is more than [`STALE_AFTER`] before `now`, or
    /// more than [`CLOCK_SKEW`] after it (a wrong clock or a restored file),
    /// so such a cache is refreshed instead of staying "fresh" forever.
    #[must_use]
    pub fn is_stale(&self, now: StoredTime) -> bool {
        now.age_at(self.updated_at) > CLOCK_SKEW || self.updated_at.age_at(now) > STALE_AFTER
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(secs: i64) -> StoredTime {
        StoredTime::from_unix_seconds(secs).unwrap()
    }

    #[test]
    fn exactly_24_hours_old_is_fresh() {
        let c = Cache::new(vec![], t(1_000));
        assert!(!c.is_stale(t(1_000 + 86_400)));
    }

    #[test]
    fn older_than_24_hours_is_stale() {
        let c = Cache::new(vec![], t(1_000));
        assert!(c.is_stale(t(1_000 + 86_401)));
    }

    #[test]
    fn small_clock_skew_is_fresh() {
        let c = Cache::new(vec![], t(1_000 + 299));
        assert!(!c.is_stale(t(1_000)));
    }

    #[test]
    fn far_future_updated_at_is_stale() {
        let c = Cache::new(vec![], t(1_000 + 301));
        assert!(c.is_stale(t(1_000)));
    }

    #[test]
    fn missing_frecency_maps_default_to_empty() {
        let c: Cache =
            serde_json::from_str(r#"{"version":1,"updated_at":"2025-01-01T00:00:00Z","repos":[]}"#)
                .unwrap();
        assert!(c.frecency.is_empty());
        assert!(c.query_frecency.is_empty());
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let c: Result<Cache, _> = serde_json::from_str(
            r#"{"version":1,"updated_at":"2025-01-01T00:00:00Z","repos":[],"extra":1}"#,
        );
        assert!(c.is_ok());
    }
}
