//! Learned query-to-repo associations and the "do we need to ask?" decision
//! (pure). See `specs/query-frecency.md` and PLAN §10 Q5.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::core::frecency::Frecency;
use crate::core::resolve::Resolution;
use crate::domain::{QueryKey, Repo, Slug, StoredTime};

/// The top history entry needs at least this many accesses to auto-resolve.
pub const MIN_QUERY_ACCESSES: u32 = 3;

/// The top history entry's score must be at least this multiple of the
/// runner-up's (the best *other* slug in the query's history).
pub const DOMINANCE_RATIO: f64 = 3.0;

/// Per-query repo frecency: `{"foo": {"github.com/acme/foobar": {...}}}`.
///
/// Reuses [`Frecency`] so there is one scoring formula.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct QueryFrecency(BTreeMap<QueryKey, Frecency>);

impl QueryFrecency {
    /// Counts one resolution of `k` to `s` at `now`. Called only after an
    /// `Auto` decision or a pick.
    pub fn record(&mut self, k: &QueryKey, s: &Slug, now: StoredTime) {
        let _ = (&mut self.0, k, s, now);
        unimplemented!("W2.1")
    }

    /// Drops slugs not in `live`, then drops queries left with no slugs.
    pub fn retain_slugs(&mut self, live: &HashSet<&Slug>) {
        let _ = (&mut self.0, live);
        unimplemented!("W2.1")
    }

    /// The history recorded for `k`, if any.
    #[must_use]
    pub fn get(&self, k: &QueryKey) -> Option<&Frecency> {
        self.0.get(k)
    }

    /// True when no query has any history.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// What the shared pipeline should do with a set of search results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// A clean resolve (one result, or one exact match). Repo index; query
    /// frecency is not recorded.
    Unique(usize),
    /// Auto-resolved from query history. Repo index; query frecency is
    /// recorded.
    Auto(usize),
    /// The user must pick from these repo indices (rank order).
    Ask(Vec<usize>),
    /// No results.
    NotFound,
}

/// Turns the caller's `core::resolve::resolve` result into a decision,
/// using query history to settle ambiguity.
///
/// `Unique` and `NotFound` pass through. `Ambiguous(results)` auto-resolves
/// when the top history entry for `key` has at least [`MIN_QUERY_ACCESSES`],
/// scores at least [`DOMINANCE_RATIO`] times the runner-up (passes when there
/// is none or it scores 0), and is among `results`; otherwise it is `Ask`
/// with every result's index in rank order. `repos` maps result indices to
/// slugs.
///
/// `--pick` is not handled here: with more than one search result the
/// pipeline asks without consulting history.
#[must_use]
pub fn decide(
    repos: &[Repo],
    resolution: Resolution,
    qf: &QueryFrecency,
    key: &QueryKey,
    now: StoredTime,
) -> Decision {
    let _ = (repos, resolution, qf, key, now);
    unimplemented!("W2.1")
}
