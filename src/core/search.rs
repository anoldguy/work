//! Tiered matching and combined scoring (pure).
//!
//! The query's slash count picks what it is compared against (name,
//! `org/name`, or the full slug). Tiers: exact 100, substring 50, fuzzy
//! `1..=49` relative to the query's self-match. The final score is the tier
//! plus repo frecency, sorted descending, ties broken by slug ascending, and
//! cut at [`RESULT_LIMIT`] (exact matches always survive the cut).

use std::borrow::Cow;

use nucleo_matcher::{Utf32Str, chars};

use crate::domain::{NameError, QueryKey, Repo, Slug};

/// The most results [`search`] returns, and so the most candidates a picker
/// or ambiguity error ever shows.
pub const RESULT_LIMIT: usize = 20;

/// What part of a slug a query is compared against, set by its slash count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Specificity {
    /// `foo`: the repo name.
    Name,
    /// `acme/foo`: `org/name`.
    OrgName,
    /// `github.com/acme/foo`: the full slug.
    Full,
}

/// A parsed repo query.
///
/// Invariant: the key is normalised (see [`QueryKey`]) and has at most two
/// slashes; `specificity` agrees with the slash count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    key: QueryKey,
    specificity: Specificity,
}

impl Query {
    /// Normalises `raw` and derives its specificity.
    ///
    /// # Errors
    ///
    /// [`NameError::Query`] when the query is empty after trimming, contains
    /// control characters, or has more than two slashes.
    pub fn parse(raw: &str) -> Result<Self, NameError> {
        let key = QueryKey::new(raw)?;
        let specificity = match key.as_str().matches('/').count() {
            0 => Specificity::Name,
            1 => Specificity::OrgName,
            2 => Specificity::Full,
            _ => {
                return Err(NameError::Query {
                    given: raw.to_owned(),
                    reason: "more than two slashes (use name, org/name, or domain/org/name)",
                });
            }
        };
        Ok(Self { key, specificity })
    }

    /// The normalised query text, also the query-frecency key.
    #[must_use]
    pub const fn key(&self) -> &QueryKey {
        &self.key
    }

    /// What the query is compared against.
    #[must_use]
    pub const fn specificity(&self) -> Specificity {
        self.specificity
    }
}

/// How a repo matched a query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchTier {
    /// Case-insensitive equality at the query's specificity.
    Exact,
    /// Substring of the level text or of the full slug.
    Substring,
    /// Subsequence match; `normalized` is in `1..=49`.
    Fuzzy {
        /// `1 + 48 * raw / self_score`, clamped to `1..=49`.
        normalized: u8,
    },
}

impl MatchTier {
    /// The tier's contribution to the combined score: 100, 50, or the
    /// normalised fuzzy score.
    #[must_use]
    pub fn score(self) -> f64 {
        match self {
            Self::Exact => 100.0,
            Self::Substring => 50.0,
            Self::Fuzzy { normalized } => f64::from(normalized),
        }
    }
}

/// One ranked hit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SearchResult {
    /// Index into the `repos` slice that was searched.
    pub index: usize,
    /// How it matched.
    pub tier: MatchTier,
    /// `tier.score()` plus the repo's frecency score.
    pub score: f64,
}

/// Ranks `repos` against `q`.
///
/// `frecency` supplies each repo's frecency score (usually
/// `|s| history.frecency.score(s, now)`). Returns results best first, cut at
/// [`RESULT_LIMIT`]. Every `Exact` hit survives the cut, so `resolve` can
/// always see the exact match: a dropped exact evicts the lowest-ranked
/// non-exact. With more than [`RESULT_LIMIT`] exact matches, the top
/// [`RESULT_LIMIT`] of them are kept, in rank order.
#[must_use]
pub fn search(repos: &[Repo], q: &Query, frecency: impl Fn(&Slug) -> f64) -> Vec<SearchResult> {
    let needle = q.key().as_str();
    let mut matcher = FuzzyMatcher::new();
    let self_score = matcher.score(needle, needle);
    let mut results: Vec<SearchResult> = repos
        .iter()
        .enumerate()
        .filter_map(|(index, repo)| {
            let tier = match_tier(&mut matcher, &repo.slug, q, self_score)?;
            Some(SearchResult {
                index,
                tier,
                score: tier.score() + frecency(&repo.slug),
            })
        })
        .collect();
    results.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| repos[a.index].slug.cmp(&repos[b.index].slug))
    });
    limit_keeping_exact(results)
}

/// Cuts ranked `results` to [`RESULT_LIMIT`]. `Exact` hits take slots first
/// (the best [`RESULT_LIMIT`] of them); the best non-exact hits fill the
/// rest. Rank order is unchanged, since it only removes entries.
fn limit_keeping_exact(mut results: Vec<SearchResult>) -> Vec<SearchResult> {
    let exact = results
        .iter()
        .filter(|r| r.tier == MatchTier::Exact)
        .count();
    let mut exact_slots = exact.min(RESULT_LIMIT);
    let mut other_slots = RESULT_LIMIT - exact_slots;
    results.retain(|r| {
        let slots = if r.tier == MatchTier::Exact {
            &mut exact_slots
        } else {
            &mut other_slots
        };
        let keep = *slots > 0;
        *slots = slots.saturating_sub(1);
        keep
    });
    results
}

/// The part of a lowercased full slug a query at `specificity` is compared
/// against: the name, `org/name`, or all of it.
fn level_text(full: &str, specificity: Specificity) -> &str {
    let suffix_after = |slashes: usize| {
        full.match_indices('/')
            .nth_back(slashes)
            .map_or(full, |(i, _)| &full[i + 1..])
    };
    match specificity {
        Specificity::Name => suffix_after(0),
        Specificity::OrgName => suffix_after(1),
        Specificity::Full => full,
    }
}

/// Classifies one slug against `q`, or `None` when nothing matches.
fn match_tier(
    matcher: &mut FuzzyMatcher,
    slug: &Slug,
    q: &Query,
    self_score: Option<u16>,
) -> Option<MatchTier> {
    let needle = q.key().as_str();
    let full = slug.to_string().to_lowercase();
    let level = level_text(&full, q.specificity());
    if level == needle {
        return Some(MatchTier::Exact);
    }
    if full.contains(needle) {
        // The level text is a suffix of the full slug, so this covers both.
        return Some(MatchTier::Substring);
    }
    let raw = matcher.score(level, needle)?;
    Some(MatchTier::Fuzzy {
        normalized: normalize(raw, self_score),
    })
}

/// `1 + 48 * raw / self_score`, clamped to `1..=49`. A missing or zero
/// self-score (which nucleo never produces for a non-empty needle) yields 1.
#[must_use]
fn normalize(raw: u16, self_score: Option<u16>) -> u8 {
    let Some(denom) = self_score.filter(|s| *s > 0) else {
        return 1;
    };
    let scaled = 1 + 48 * u32::from(raw) / u32::from(denom);
    u8::try_from(scaled.clamp(1, 49)).unwrap_or(49)
}

/// `list --filter`: case-insensitive substring match on the full slug, with
/// no fuzzy tier and no frecency. The needle is trimmed like a query.
/// Returns indices into `repos` in slug order.
#[must_use]
pub fn filter(repos: &[Repo], needle: &str) -> Vec<usize> {
    let needle = needle.trim().to_lowercase();
    let mut hits: Vec<usize> = repos
        .iter()
        .enumerate()
        .filter(|(_, repo)| repo.slug.to_string().to_lowercase().contains(&needle))
        .map(|(index, _)| index)
        .collect();
    hits.sort_by(|&a, &b| repos[a].slug.cmp(&repos[b].slug).then(a.cmp(&b)));
    hits
}

/// Thin wrapper over nucleo-matcher. It owns the matcher and its UTF-32
/// conversion buffers, so they are allocated once per search rather than
/// once per repo.
#[derive(Debug)]
pub struct FuzzyMatcher {
    matcher: nucleo_matcher::Matcher,
    hay_buf: Vec<char>,
    needle_buf: Vec<char>,
}

impl FuzzyMatcher {
    /// A matcher with nucleo's default configuration (which ignores case).
    #[must_use]
    pub fn new() -> Self {
        Self {
            matcher: nucleo_matcher::Matcher::new(nucleo_matcher::Config::DEFAULT),
            hay_buf: Vec::new(),
            needle_buf: Vec::new(),
        }
    }

    /// Raw subsequence score of `needle` in `haystack`, or `None` when it does
    /// not match. Both are compared case-insensitively.
    pub fn score(&mut self, haystack: &str, needle: &str) -> Option<u16> {
        // nucleo folds the haystack's case and accents but expects a needle
        // that is already folded; haystack case also changes its camel-case
        // bonus, so both are lowercased. `search` passes lowercase text, so
        // ASCII input allocates nothing there.
        let haystack = lowercase(haystack);
        let needle: Cow<'_, str> = if needle.is_ascii() {
            lowercase(needle)
        } else {
            Cow::Owned(
                needle
                    .chars()
                    .map(|c| chars::normalize(chars::to_lower_case(c)))
                    .collect(),
            )
        };
        self.matcher.fuzzy_match(
            Utf32Str::new(&haystack, &mut self.hay_buf),
            Utf32Str::new(&needle, &mut self.needle_buf),
        )
    }
}

/// `s` lowercased, borrowing when it has no uppercase characters.
fn lowercase(s: &str) -> Cow<'_, str> {
    if s.chars().any(char::is_uppercase) {
        Cow::Owned(s.to_lowercase())
    } else {
        Cow::Borrowed(s)
    }
}

impl Default for FuzzyMatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use proptest::prelude::*;

    use super::{
        FuzzyMatcher, MatchTier, Query, RESULT_LIMIT, Specificity, filter, normalize, search,
    };
    use crate::core::resolve::{Resolution, resolve};
    use crate::domain::{CloneUrl, HtmlUrl, NameError, Repo, Slug};

    fn repo(slug: &str) -> Repo {
        Repo {
            slug: slug.parse().expect("valid slug"),
            clone_url: CloneUrl::try_from(format!("https://{slug}.git")).expect("clone url"),
            html_url: HtmlUrl::try_from(format!("https://{slug}")).expect("html url"),
        }
    }

    fn repos(slugs: &[&str]) -> Vec<Repo> {
        slugs.iter().copied().map(repo).collect()
    }

    fn q(raw: &str) -> Query {
        Query::parse(raw).expect("valid query")
    }

    fn no_frecency(_: &Slug) -> f64 {
        0.0
    }

    /// The searched slugs, best first.
    fn ranked(rs: &[Repo], query: &Query, f: impl Fn(&Slug) -> f64) -> Vec<String> {
        search(rs, query, f)
            .iter()
            .map(|r| rs[r.index].slug.to_string())
            .collect()
    }

    #[test]
    fn parse_sets_specificity_from_slash_count() {
        assert_eq!(q("foo").specificity(), Specificity::Name);
        assert_eq!(q("acme/foo").specificity(), Specificity::OrgName);
        assert_eq!(q("github.com/acme/foo").specificity(), Specificity::Full);
    }

    #[test]
    fn parse_normalises_the_key() {
        assert_eq!(q("  Acme/Foo ").key().as_str(), "acme/foo");
    }

    #[test]
    fn parse_rejects_more_than_two_slashes() {
        assert_matches!(Query::parse("a/b/c/d"), Err(NameError::Query { .. }));
    }

    #[test]
    fn parse_rejects_empty() {
        assert_matches!(Query::parse("  "), Err(NameError::Query { .. }));
    }

    #[test]
    fn exact_is_judged_at_the_query_specificity() {
        let rs = repos(&["github.com/acme/foo", "github.com/acme/foobar"]);
        let results = search(&rs, &q("foo"), no_frecency);
        assert_matches!(results[0].tier, MatchTier::Exact);
        assert_eq!(results[0].index, 0);
        assert_matches!(results[1].tier, MatchTier::Substring);

        let org = search(&rs, &q("acme/foo"), no_frecency);
        assert_matches!(org[0].tier, MatchTier::Exact);
        assert_eq!(org[0].index, 0);

        let full = search(&rs, &q("github.com/acme/foobar"), no_frecency);
        assert_matches!(full[0].tier, MatchTier::Exact);
        assert_eq!(full[0].index, 1);
    }

    #[test]
    fn name_query_is_not_exact_on_the_org() {
        let rs = repos(&["github.com/foo/bar"]);
        let results = search(&rs, &q("foo"), no_frecency);
        assert_matches!(results[..], [r] if r.tier == MatchTier::Substring);
    }

    #[test]
    fn matching_is_case_insensitive() {
        let rs = repos(&["github.com/Acme/FooBar"]);
        let results = search(&rs, &q("acme/foobar"), no_frecency);
        assert_matches!(results[..], [r] if r.tier == MatchTier::Exact);
        let upper = search(&rs, &q("FOOB"), no_frecency);
        assert_matches!(upper[..], [r] if r.tier == MatchTier::Substring);
    }

    #[test]
    fn subsequence_is_fuzzy_and_non_matches_are_excluded() {
        let rs = repos(&["github.com/acme/frobnicator", "github.com/acme/zzz"]);
        let results = search(&rs, &q("fbn"), no_frecency);
        assert_matches!(
            results[..],
            [r] if r.index == 0 && matches!(r.tier, MatchTier::Fuzzy { normalized: 1..=49 })
        );
    }

    #[test]
    fn combined_score_is_tier_plus_frecency() {
        let rs = repos(&["github.com/acme/foobar"]);
        let results = search(&rs, &q("foo"), |_| 7.5);
        assert_matches!(results[..], [r] if (r.score - 57.5).abs() < f64::EPSILON);
    }

    #[test]
    fn exact_without_frecency_beats_substring_with_moderate_frecency() {
        let rs = repos(&["github.com/acme/foobar", "github.com/acme/foo"]);
        let order = ranked(&rs, &q("foo"), |s| {
            if s.name().as_str() == "foobar" {
                40.0
            } else {
                0.0
            }
        });
        assert_eq!(order, ["github.com/acme/foo", "github.com/acme/foobar"]);
    }

    #[test]
    fn heavy_usage_promotes_a_substring_above_an_exact_match() {
        let rs = repos(&["github.com/other/foo", "github.com/acme/foo-app"]);
        let order = ranked(&rs, &q("foo"), |s| {
            if s.org().as_str() == "acme" {
                80.0
            } else {
                0.0
            }
        });
        assert_eq!(order, ["github.com/acme/foo-app", "github.com/other/foo"]);
    }

    #[test]
    fn ties_break_by_slug_ascending() {
        let rs = repos(&[
            "github.com/zeta/foo",
            "github.com/alpha/foo",
            "ghe.example.com/alpha/foo",
        ]);
        let order = ranked(&rs, &q("foo"), no_frecency);
        assert_eq!(
            order,
            [
                "ghe.example.com/alpha/foo",
                "github.com/alpha/foo",
                "github.com/zeta/foo",
            ]
        );
    }

    #[test]
    fn results_are_limited() {
        let slugs: Vec<String> = (0..30)
            .map(|i| format!("github.com/acme/foo{i:02}"))
            .collect();
        let rs: Vec<Repo> = slugs.iter().map(|s| repo(s)).collect();
        assert_eq!(search(&rs, &q("foo"), no_frecency).len(), RESULT_LIMIT);
    }

    #[test]
    fn substring_of_the_full_slug_counts() {
        let rs = repos(&["github.com/acme/widget"]);
        let results = search(&rs, &q("cme/wid"), no_frecency);
        assert_matches!(results[..], [r] if r.tier == MatchTier::Substring);
    }

    #[test]
    fn fuzzy_matcher_is_case_insensitive() {
        let mut m = FuzzyMatcher::new();
        assert!(m.score("FooBar", "fb").is_some());
        assert_eq!(m.score("FooBar", "FB"), m.score("foobar", "fb"));
        assert_eq!(m.score("abc", "xyz"), None);
    }

    #[test]
    fn filter_is_substring_only_in_slug_order() {
        let rs = repos(&[
            "github.com/zeta/foo",
            "github.com/acme/FOOBAR",
            "github.com/acme/frobnicate-o-o",
            "github.com/acme/bar",
        ]);
        assert_eq!(filter(&rs, "Foo"), [1, 0]);
        assert_eq!(filter(&rs, "nomatch"), Vec::<usize>::new());
    }

    #[test]
    fn normalize_handles_missing_self_score() {
        assert_eq!(normalize(100, None), 1);
        assert_eq!(normalize(100, Some(0)), 1);
        assert_eq!(normalize(100, Some(100)), 49);
    }

    #[test]
    fn the_cut_keeps_an_exact_match_outranked_by_frecency() {
        let mut rs: Vec<Repo> = (0..25)
            .map(|i| repo(&format!("github.com/acme/foo-{i:02}")))
            .collect();
        rs.push(repo("github.com/zzz/foo"));
        let busy = |s: &Slug| {
            if s.org().as_str() == "acme" {
                60.0
            } else {
                0.0
            }
        };
        let results = search(&rs, &q("foo"), busy);
        assert_eq!(results.len(), RESULT_LIMIT);
        assert_matches!(
            results.last(),
            Some(r) if r.index == 25 && r.tier == MatchTier::Exact
        );
        assert!(results.windows(2).all(|w| w[0].score >= w[1].score));
        assert_matches!(resolve(results), Resolution::Unique(25));
    }

    #[test]
    fn more_exact_matches_than_the_limit_keep_the_top_exact_ones() {
        let rs: Vec<Repo> = (0..25)
            .map(|i| repo(&format!("github.com/org{i:02}/foo")))
            .chain([repo("github.com/acme/foobar")])
            .collect();
        let busy = |s: &Slug| {
            if s.org().as_str() == "org24" {
                9.0
            } else {
                0.0
            }
        };
        let results = search(&rs, &q("foo"), busy);
        assert_eq!(results.len(), RESULT_LIMIT);
        assert!(results.iter().all(|r| r.tier == MatchTier::Exact));
        let indices: Vec<usize> = results.iter().map(|r| r.index).collect();
        let expected: Vec<usize> = [24].into_iter().chain(0..19).collect();
        assert_eq!(indices, expected);
    }

    #[test]
    fn a_name_query_also_matches_org_and_domain_as_substring() {
        // Deliberate: the spec makes substring-of-full-slug a match, so a
        // name query hits every repo in an org or domain with that text.
        let rs = repos(&["github.com/acme/widget", "ghe.example.com/other/thing"]);
        let by_org = search(&rs, &q("acme"), no_frecency);
        assert_matches!(by_org[..], [r] if r.index == 0 && r.tier == MatchTier::Substring);
        let by_domain = search(&rs, &q("github"), no_frecency);
        assert_matches!(by_domain[..], [r] if r.index == 0 && r.tier == MatchTier::Substring);
    }

    #[test]
    fn within_a_tier_frecency_orders_results() {
        let rs = repos(&["github.com/acme/foo-a", "github.com/acme/foo-b"]);
        let order = ranked(&rs, &q("foo"), |s| {
            if s.name().as_str() == "foo-b" {
                5.0
            } else {
                1.0
            }
        });
        assert_eq!(order, ["github.com/acme/foo-b", "github.com/acme/foo-a"]);
    }

    #[test]
    fn fuzzy_ties_break_by_slug_ascending() {
        let rs = repos(&["github.com/zeta/frobnicate", "github.com/alpha/frobnicate"]);
        let results = search(&rs, &q("fbn"), no_frecency);
        assert_matches!(
            results[..],
            [a, b] if a.index == 1 && b.index == 0
                && matches!(a.tier, MatchTier::Fuzzy { .. })
                && a.tier == b.tier
        );
    }

    #[test]
    fn filter_trims_its_needle() {
        let rs = repos(&["github.com/acme/foo", "github.com/acme/bar"]);
        assert_eq!(filter(&rs, "  FOO "), [0]);
    }

    #[test]
    fn fuzzy_matcher_folds_case_and_accents_across_calls() {
        let mut m = FuzzyMatcher::new();
        let first = m.score("Ünïcödé-repo", "ÜR");
        assert!(first.is_some());
        assert_eq!(m.score("Ünïcödé-repo", "ür"), first);
        assert_eq!(m.score("plain", "ür"), None);
    }

    fn name_strategy() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9-]{0,8}"
    }

    fn acme_repos(names: &[String]) -> Vec<Repo> {
        names
            .iter()
            .map(|n| repo(&format!("github.com/acme/{n}")))
            .collect()
    }

    proptest! {
        #[test]
        fn normalized_fuzzy_score_stays_in_bounds(raw in any::<u16>(), self_score in any::<u16>()) {
            let n = normalize(raw, Some(self_score));
            prop_assert!((1..=49).contains(&n));
        }

        #[test]
        fn search_fuzzy_bounds_hold(
            names in prop::collection::vec(name_strategy(), 1..30),
            query in "[a-z0-9-]{1,5}",
        ) {
            let rs = acme_repos(&names);
            for r in search(&rs, &q(&query), no_frecency) {
                if let MatchTier::Fuzzy { normalized } = r.tier {
                    prop_assert!((1..=49).contains(&normalized));
                }
            }
        }

        #[test]
        fn scoring_is_deterministic(
            names in prop::collection::vec(name_strategy(), 0..30),
            query in "[a-z0-9-]{1,5}",
        ) {
            let rs = acme_repos(&names);
            let query = q(&query);
            let freq = |s: &Slug| f64::from(u8::try_from(s.name().as_str().len()).unwrap_or(0));
            prop_assert_eq!(search(&rs, &query, freq), search(&rs, &query, freq));
        }

        #[test]
        fn an_exact_match_is_always_in_the_results(
            names in prop::collection::vec(name_strategy(), 0..40),
            target in name_strategy(),
        ) {
            let mut rs = acme_repos(&names);
            rs.push(repo(&format!("github.com/zzz/{target}")));
            let exact_index = rs.len() - 1;
            let results = search(&rs, &q(&target), no_frecency);
            prop_assert!(results.len() <= RESULT_LIMIT);
            prop_assert!(
                results.iter().any(|r| r.index == exact_index && r.tier == MatchTier::Exact)
            );
        }

        #[test]
        fn an_exact_match_survives_any_frecency(
            names in prop::collection::vec(name_strategy(), 0..40),
            target in name_strategy(),
            weights in prop::collection::vec(0.0f64..500.0, 41),
        ) {
            let mut rs = acme_repos(&names);
            rs.push(repo(&format!("github.com/zzz/{target}")));
            let exact_index = rs.len() - 1;
            let freq = |s: &Slug| {
                rs.iter().position(|r| &r.slug == s).map_or(0.0, |i| weights[i])
            };
            let results = search(&rs, &q(&target), freq);
            prop_assert!(results.windows(2).all(|w| w[0].score >= w[1].score));
            prop_assert!(results.iter().any(|r| r.index == exact_index));
            let exact = results.iter().filter(|r| r.tier == MatchTier::Exact).count();
            prop_assert!(results.len() <= RESULT_LIMIT);
            prop_assert!(exact >= 1);
        }
    }
}
