//! The cache data and its staleness rule (pure).

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::domain::{Repo, StoredTime};

/// Version written to new caches; a file with any other version is corrupt.
pub const CACHE_VERSION: u32 = 2;

/// Age after which a cache is stale.
pub const STALE_AFTER: Duration = Duration::from_hours(24);

/// How far in the future `updated_at` may be before the cache counts as stale.
pub const CLOCK_SKEW: Duration = Duration::from_mins(5);

/// File name inside the cache directory.
pub const CACHE_FILE: &str = "cache.json";

/// The contents of `cache.json`. Unknown fields are ignored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cache {
    /// Always [`CACHE_VERSION`] when written by this build.
    pub version: u32,
    /// When the repo list was last refreshed.
    pub updated_at: StoredTime,
    /// Every cached repo, de-duplicated by slug.
    pub repos: Vec<Repo>,
}

impl Cache {
    /// A cache of `repos` updated at `now`.
    #[cfg(test)]
    #[must_use]
    pub fn new(repos: Vec<Repo>, now: StoredTime) -> Self {
        Self {
            version: CACHE_VERSION,
            updated_at: now,
            repos,
        }
    }

    /// True when `updated_at` is more than [`STALE_AFTER`] before `now`, or
    /// more than [`CLOCK_SKEW`] after it, so a future timestamp cannot keep a
    /// cache fresh forever.
    #[must_use]
    pub fn is_stale(&self, now: StoredTime) -> bool {
        now.age_at(self.updated_at) > CLOCK_SKEW || self.updated_at.age_at(now) > STALE_AFTER
    }
}
