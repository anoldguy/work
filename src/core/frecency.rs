//! Repo frecency: access count times a recency multiplier (pure).

use std::collections::BTreeMap;

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
///
/// ```
/// use work::core::frecency::recency_multiplier;
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

const SECS_PER_DAY: u64 = 86_400;

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

#[cfg(test)]
mod tests {

    use super::*;

    const DAY: i64 = 86_400;

    fn slug(s: &str) -> Slug {
        Slug::try_from(s.to_owned()).unwrap()
    }

    fn t(secs: i64) -> StoredTime {
        StoredTime::from_unix_seconds(secs).unwrap()
    }

    #[test]
    fn score_uses_whole_days_at_boundaries() {
        let s = slug("github.com/a/b");
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
        assert_eq!(f.score(&slug("github.com/a/none"), t(0)), 0.0);
    }
}
