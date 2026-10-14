//! Tiered matching and combined scoring (pure).
//!
//! The query's slash count picks what it is compared against (name,
//! `org/name`, or the full slug). Tiers: exact 100, substring 50, fuzzy
//! `1..=49` relative to the query's self-match. The final score is the tier
//! plus repo frecency, sorted descending, ties broken by slug ascending, and
//! cut at [`RESULT_LIMIT`].

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
        let _ = raw;
        unimplemented!("W2.2")
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
/// `|s| cache.frecency.score(s, now)`). Returns at most [`RESULT_LIMIT`]
/// results, best first.
#[must_use]
pub fn search(repos: &[Repo], q: &Query, frecency: impl Fn(&Slug) -> f64) -> Vec<SearchResult> {
    let _ = (repos, q, frecency);
    unimplemented!("W2.2")
}

/// `list --filter`: case-insensitive substring match on the full slug, with
/// no fuzzy tier and no frecency. Returns indices into `repos` in slug order.
#[must_use]
pub fn filter(repos: &[Repo], needle: &str) -> Vec<usize> {
    let _ = (repos, needle);
    unimplemented!("W2.2")
}

/// Thin wrapper over nucleo-matcher, so the matcher's buffers are allocated
/// once per search rather than once per repo.
#[derive(Debug)]
pub struct FuzzyMatcher {
    matcher: nucleo_matcher::Matcher,
}

impl FuzzyMatcher {
    /// A matcher with nucleo's default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self {
            matcher: nucleo_matcher::Matcher::new(nucleo_matcher::Config::DEFAULT),
        }
    }

    /// Raw subsequence score of `needle` in `haystack`, or `None` when it does
    /// not match. Both are compared case-insensitively.
    pub fn score(&mut self, haystack: &str, needle: &str) -> Option<u16> {
        let _ = (&mut self.matcher, haystack, needle);
        unimplemented!("W2.2")
    }
}

impl Default for FuzzyMatcher {
    fn default() -> Self {
        Self::new()
    }
}
