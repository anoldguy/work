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

    /// True when `updated_at` is more than [`STALE_AFTER`] before `now`.
    #[must_use]
    pub fn is_stale(&self, now: StoredTime) -> bool {
        let _ = (self, now);
        unimplemented!("W2.5")
    }
}
