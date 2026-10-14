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

/// Relative slack that absorbs float rounding in the dominance comparison
/// (`9 * 0.2` vs `3 * (3 * 0.2)` differ in the last bit).
const DOMINANCE_EPSILON: f64 = 1e-9;

/// True when `score` is at least [`DOMINANCE_RATIO`] times `runner_up`,
/// robust to rounding at exactly the ratio.
fn dominates(score: f64, runner_up: f64) -> bool {
    score >= DOMINANCE_RATIO * runner_up * (1.0 - DOMINANCE_EPSILON)
}

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
        if let Some(history) = self.0.get_mut(k) {
            history.record(s, now);
        } else {
            let mut history = Frecency::default();
            history.record(s, now);
            self.0.insert(k.clone(), history);
        }
    }

    /// Drops slugs not in `live`, then drops queries left with no slugs.
    pub fn retain_slugs(&mut self, live: &HashSet<&Slug>) {
        for history in self.0.values_mut() {
            history.retain_slugs(live);
        }
        self.0.retain(|_, history| !history.is_empty());
    }

    /// The history recorded for `k`, if any.
    #[must_use]
    pub fn get(&self, k: &QueryKey) -> Option<&Frecency> {
        self.0.get(k)
    }

    /// True when no query has any history (tests).
    #[cfg(test)]
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
    let results = match resolution {
        Resolution::Unique(i) => return Decision::Unique(i),
        Resolution::NotFound => return Decision::NotFound,
        Resolution::Ambiguous(results) if results.is_empty() => return Decision::NotFound,
        Resolution::Ambiguous(results) => results,
    };
    let auto = qf.get(key).and_then(|history| {
        let (Some((slug, data, score)), runner_up) = history.top_two(now) else {
            return None;
        };
        let dominant = data.access_count >= MIN_QUERY_ACCESSES
            && (runner_up <= 0.0 || dominates(score, runner_up));
        if !dominant {
            return None;
        }
        results
            .iter()
            .find(|r| repos.get(r.index).is_some_and(|repo| repo.slug == *slug))
            .map(|r| r.index)
    });
    auto.map_or_else(
        || Decision::Ask(results.iter().map(|r| r.index).collect()),
        Decision::Auto,
    )
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::core::search::{MatchTier, SearchResult};
    use crate::domain::{CloneUrl, HtmlUrl};

    const DAY: i64 = 86_400;
    const NOW: i64 = 1_000 * DAY;

    fn t(secs: i64) -> StoredTime {
        StoredTime::from_unix_seconds(secs).unwrap()
    }

    fn repo(name: &str) -> Repo {
        Repo {
            slug: Slug::try_from(format!("github.com/acme/{name}")).unwrap(),
            clone_url: CloneUrl::try_from(format!("https://github.com/acme/{name}.git")).unwrap(),
            html_url: HtmlUrl::try_from(format!("https://github.com/acme/{name}")).unwrap(),
        }
    }

    fn repos() -> Vec<Repo> {
        ["foobar", "foobaz", "foo-legacy"].map(repo).into()
    }

    fn ambiguous(indices: &[usize]) -> Resolution {
        Resolution::Ambiguous(
            indices
                .iter()
                .map(|&index| SearchResult {
                    index,
                    tier: MatchTier::Substring,
                    score: 50.0,
                })
                .collect(),
        )
    }

    fn key() -> QueryKey {
        QueryKey::new("Foo ").unwrap()
    }

    fn history(entries: &[(&str, u32, i64)]) -> QueryFrecency {
        let mut qf = QueryFrecency::default();
        for (name, n, at) in entries {
            for _ in 0..*n {
                qf.record(&key(), &repo(name).slug, t(*at));
            }
        }
        qf
    }

    fn run(r: Resolution, qf: &QueryFrecency) -> Decision {
        decide(&repos(), r, qf, &key(), t(NOW))
    }

    #[test]
    fn unique_and_not_found_bypass_history() {
        let qf = history(&[("foobar", 10, NOW)]);
        assert_matches!(run(Resolution::Unique(1), &qf), Decision::Unique(1));
        assert_matches!(run(Resolution::NotFound, &qf), Decision::NotFound);
    }

    #[test]
    fn no_history_asks() {
        let d = run(ambiguous(&[0, 1]), &QueryFrecency::default());
        assert_eq!(d, Decision::Ask(vec![0, 1]));
    }

    #[test]
    fn auto_resolves_at_minimum_accesses() {
        let qf = history(&[("foobaz", 3, NOW)]);
        assert_eq!(run(ambiguous(&[0, 1]), &qf), Decision::Auto(1));
    }

    #[test]
    fn below_minimum_accesses_asks() {
        let qf = history(&[("foobaz", 2, NOW)]);
        assert_eq!(run(ambiguous(&[0, 1]), &qf), Decision::Ask(vec![0, 1]));
    }

    #[test]
    fn dominance_exactly_three_times_passes() {
        let qf = history(&[("foobar", 9, NOW), ("foobaz", 3, NOW)]);
        assert_eq!(run(ambiguous(&[0, 1]), &qf), Decision::Auto(0));
    }

    #[test]
    fn dominance_exactly_three_times_passes_in_every_tier() {
        for days in [0, 7, 8, 30, 31, 40, 90, 91, 100] {
            let at = NOW - days * DAY;
            let qf = history(&[("foobar", 9, at), ("foobaz", 3, at)]);
            assert_eq!(run(ambiguous(&[0, 1]), &qf), Decision::Auto(0), "{days}d");
            let qf = history(&[("foobar", 8, at), ("foobaz", 3, at)]);
            assert_eq!(
                run(ambiguous(&[0, 1]), &qf),
                Decision::Ask(vec![0, 1]),
                "{days}d"
            );
        }
    }

    #[test]
    fn dominance_across_tiers_at_exact_ratio() {
        // 9 at 40d = 1.8 vs 12 at 100d = 0.6.
        let qf = history(&[
            ("foobar", 9, NOW - 40 * DAY),
            ("foo-legacy", 12, NOW - 100 * DAY),
        ]);
        assert_eq!(run(ambiguous(&[0, 1]), &qf), Decision::Auto(0));
    }

    proptest::proptest! {
        #[test]
        fn exact_multiples_pass(
            runner in 1u32..50,
            days in 0i64..200,
            runner_days in 0i64..200,
        ) {
            // Top has exactly 3x the runner-up's score iff its count is
            // 3 * runner * (runner multiplier / top multiplier).
            let (top_mult, runner_mult) = (mult(days), mult(runner_days));
            let top_count = (3.0 * f64::from(runner) * runner_mult / top_mult).round();
            let exact = (top_count * top_mult - 3.0 * f64::from(runner) * runner_mult).abs() < 1e-12;
            proptest::prop_assume!(exact && (3.0..10_000.0).contains(&top_count));
            #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "bounded by the assume above")]
            let top_count = top_count as u32;
            let qf = history(&[
                ("foobar", top_count, NOW - days * DAY),
                ("foobaz", runner, NOW - runner_days * DAY),
            ]);
            proptest::prop_assert_eq!(run(ambiguous(&[0, 1]), &qf), Decision::Auto(0));
        }
    }

    fn mult(days: i64) -> f64 {
        crate::core::frecency::recency_multiplier(days.unsigned_abs())
    }

    #[test]
    fn insufficient_dominance_asks() {
        let qf = history(&[("foobar", 8, NOW), ("foobaz", 3, NOW)]);
        assert_eq!(run(ambiguous(&[0, 1]), &qf), Decision::Ask(vec![0, 1]));
    }

    #[test]
    fn runner_up_outside_candidates_still_counts() {
        // foo-legacy (index 2) is not a candidate: 9.0 < 3 * 4.0.
        let qf = history(&[("foobar", 9, NOW), ("foo-legacy", 4, NOW)]);
        assert_eq!(run(ambiguous(&[0, 1]), &qf), Decision::Ask(vec![0, 1]));
    }

    #[test]
    fn stale_runner_up_does_not_block() {
        let qf = history(&[("foobar", 3, NOW), ("foo-legacy", 10, NOW - 100 * DAY)]);
        assert_eq!(run(ambiguous(&[0, 1]), &qf), Decision::Auto(0));
    }

    #[test]
    fn top_entry_must_be_a_candidate() {
        let qf = history(&[("foo-legacy", 10, NOW), ("foobar", 3, NOW)]);
        assert_eq!(run(ambiguous(&[0, 1]), &qf), Decision::Ask(vec![0, 1]));
    }

    #[test]
    fn gc_removes_dead_slugs_and_empty_queries() {
        let mut qf = history(&[("foobar", 1, NOW), ("foobaz", 1, NOW)]);
        let other = QueryKey::new("bar").unwrap();
        qf.record(&other, &repo("foobaz").slug, t(NOW));
        let bar = repo("foobar").slug;
        qf.retain_slugs(&HashSet::from([&bar]));
        assert!(qf.get(&key()).is_some_and(|f| f.get(&bar).is_some()));
        assert_eq!(
            qf.get(&key()).map(|f| f.get(&repo("foobaz").slug)),
            Some(None)
        );
        assert!(qf.get(&other).is_none());
        qf.retain_slugs(&HashSet::new());
        assert!(qf.is_empty());
    }
}
