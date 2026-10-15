//! Learned query-to-repo associations and the "do we need to ask?" decision
//! (pure).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::core::frecency::{Frecency, FrecencyData};
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

/// What the shared pipeline should do with a set of search results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// A clean resolve (one result, or one exact match), holding the repo
    /// index.
    Unique(usize),
    /// Auto-resolved from query history, holding the repo index.
    Auto(usize),
    /// The user must pick from these repo indices (rank order).
    Ask(Vec<usize>),
    /// No results.
    NotFound,
}

/// Turns a [`Resolution`] into a decision, using query history to settle
/// ambiguity.
///
/// `Unique` and `NotFound` pass through. `Ambiguous(results)` auto-resolves
/// when the top history entry for `key` has at least [`MIN_QUERY_ACCESSES`],
/// scores at least [`DOMINANCE_RATIO`] times the runner-up (passes when there
/// is none or it scores 0), and is among `results`; otherwise it is `Ask`
/// with every result's index in rank order. `repos` maps result indices to
/// slugs.
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
    use super::*;
    use crate::core::search::{MatchTier, SearchResult};
    use crate::domain::{CloneUrl, HtmlUrl};

    const DAY: i64 = 86_400;
    const NOW: i64 = 1_000 * DAY;
    const NAMES: [&str; 3] = ["foobar", "foo-legacy", "foobaz"];

    /// `(name, picks, days ago)` for one query.
    type History<'a> = &'a [(&'a str, u32, i64)];

    fn t(secs: i64) -> StoredTime {
        StoredTime::from_unix_seconds(secs).expect("time")
    }

    fn repo(name: &str) -> Repo {
        Repo {
            slug: Slug::try_from(format!("github.com/acme/{name}")).expect("slug"),
            clone_url: CloneUrl::try_from(format!("https://github.com/acme/{name}.git"))
                .expect("clone url"),
            html_url: HtmlUrl::try_from(format!("https://github.com/acme/{name}"))
                .expect("html url"),
        }
    }

    /// Decides `foo` over the candidates `names` (a subset of [`NAMES`])
    /// with `(name, picks, days ago)` as its history.
    fn decide_foo(names: &[&str], history: History<'_>) -> Decision {
        let repos: Vec<Repo> = NAMES.map(repo).into();
        let key = QueryKey::new("foo").expect("key");
        let mut qf = QueryFrecency::default();
        for (name, picks, days) in history {
            for _ in 0..*picks {
                qf.record(&key, &repo(name).slug, t(NOW - days * DAY));
            }
        }
        let index = |name: &&str| NAMES.iter().position(|n| n == name).expect("known");
        let results = names
            .iter()
            .map(|name| SearchResult {
                index: index(name),
                tier: MatchTier::Substring,
                score: 50.0,
            })
            .collect();
        decide(&repos, Resolution::Ambiguous(results), &qf, &key, t(NOW))
    }

    #[test]
    fn worked_examples_a_to_e() {
        let both = ["foobar", "foo-legacy"];
        let ask = Decision::Ask(vec![0, 1]);
        let scenarios: [(&str, &[&str], History<'_>, Decision); 5] = [
            (
                "A: 2.25x is not dominant",
                &both,
                &[("foobar", 9, 0), ("foo-legacy", 4, 0)],
                ask.clone(),
            ),
            (
                "B: 11.25x",
                &both,
                &[("foobar", 9, 0), ("foo-legacy", 4, 60)],
                Decision::Auto(0),
            ),
            (
                "C: below the minimum picks",
                &both,
                &[("foobar", 2, 0)],
                ask,
            ),
            (
                "D: no runner-up",
                &both,
                &[("foobar", 3, 0)],
                Decision::Auto(0),
            ),
            (
                "E: top entry not a candidate",
                &["foobar", "foobaz"],
                &[("foo-legacy", 10, 0), ("foobar", 3, 0)],
                Decision::Ask(vec![0, 2]),
            ),
        ];
        for (scenario, candidates, history, expected) in scenarios {
            assert_eq!(decide_foo(candidates, history), expected, "{scenario}");
        }
    }

    #[test]
    fn a_runner_up_outside_the_results_still_counts() {
        let decision = decide_foo(
            &["foobar", "foobaz"],
            &[("foobar", 9, 0), ("foo-legacy", 4, 0)],
        );
        assert_eq!(decision, Decision::Ask(vec![0, 2]));
    }

    #[test]
    fn a_unique_result_ignores_history() {
        let repos: Vec<Repo> = NAMES.map(repo).into();
        let key = QueryKey::new("foo").expect("key");
        let decision = decide(
            &repos,
            Resolution::Unique(2),
            &QueryFrecency::default(),
            &key,
            t(NOW),
        );
        assert_eq!(decision, Decision::Unique(2));
    }
}
