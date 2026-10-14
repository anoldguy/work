//! Repo frecency: access count times a recency multiplier (pure).

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::domain::{Slug, StoredTime};

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
#[must_use]
pub const fn recency_multiplier(age_days: u64) -> f64 {
    match age_days {
        0..=7 => 1.0,
        8..=30 => 0.5,
        31..=90 => 0.2,
        _ => 0.05,
    }
}

/// Frecency per repo, keyed by slug. Serialised as a plain JSON object
/// (`{"github.com/acme/foo": {"access_count": 3, "last_access": "..."}}`).
///
/// Keys compare exactly (case-sensitive), as the API returned them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Frecency(BTreeMap<Slug, FrecencyData>);

impl Frecency {
    /// Counts one access to `s` at `now`.
    pub fn record(&mut self, s: &Slug, now: StoredTime) {
        let _ = (&mut self.0, s, now);
        unimplemented!("W2.1")
    }

    /// `access_count * recency_multiplier(age in days)`, or 0 for a slug with
    /// no history.
    #[must_use]
    pub fn score(&self, s: &Slug, now: StoredTime) -> f64 {
        let _ = (&self.0, s, now);
        unimplemented!("W2.1")
    }

    /// The highest-scoring entry (with its score), and the best score among
    /// all *other* entries (0 when there is none). Ties break by slug
    /// ascending.
    #[must_use]
    pub fn top_two(&self, now: StoredTime) -> (Option<(&Slug, &FrecencyData, f64)>, f64) {
        let _ = (&self.0, now);
        unimplemented!("W2.1")
    }

    /// Drops every entry whose slug is not in `live`.
    pub fn retain_slugs(&mut self, live: &HashSet<&Slug>) {
        let _ = (&mut self.0, live);
        unimplemented!("W2.1")
    }

    /// The raw entry for `s`, if any.
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
