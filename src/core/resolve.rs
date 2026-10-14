//! Turns ranked search results into zero, one, or an ambiguous set (pure).

use crate::core::search::{MatchTier, SearchResult};

/// The outcome of resolving a search without any history or interaction.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    /// Exactly one result, or exactly one `Exact` among several. Holds the
    /// repo index.
    Unique(usize),
    /// No results.
    NotFound,
    /// Several results and no single exact match, in rank order.
    Ambiguous(Vec<SearchResult>),
}

/// Applies the resolution rules to `results` (already ranked by `search`).
#[must_use]
pub fn resolve(results: Vec<SearchResult>) -> Resolution {
    match results.as_slice() {
        [] => Resolution::NotFound,
        [only] => Resolution::Unique(only.index),
        many => {
            let mut exact = many.iter().filter(|r| r.tier == MatchTier::Exact);
            match (exact.next(), exact.next()) {
                (Some(hit), None) => Resolution::Unique(hit.index),
                _ => Resolution::Ambiguous(results),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::{Resolution, resolve};
    use crate::core::search::{MatchTier, SearchResult};

    fn hit(index: usize, tier: MatchTier) -> SearchResult {
        SearchResult {
            index,
            tier,
            score: tier.score(),
        }
    }

    #[test]
    fn no_results_is_not_found() {
        assert_matches!(resolve(Vec::new()), Resolution::NotFound);
    }

    #[test]
    fn a_single_result_is_unique_whatever_its_tier() {
        let fuzzy = MatchTier::Fuzzy { normalized: 3 };
        assert_matches!(resolve(vec![hit(7, fuzzy)]), Resolution::Unique(7));
    }

    #[test]
    fn one_exact_among_several_is_unique() {
        let results = vec![
            hit(0, MatchTier::Substring),
            hit(4, MatchTier::Exact),
            hit(2, MatchTier::Fuzzy { normalized: 10 }),
        ];
        assert_matches!(resolve(results), Resolution::Unique(4));
    }

    #[test]
    fn two_exact_matches_are_ambiguous_in_rank_order() {
        let results = vec![hit(3, MatchTier::Exact), hit(1, MatchTier::Exact)];
        assert_matches!(
            resolve(results),
            Resolution::Ambiguous(ref r) if r.iter().map(|h| h.index).eq([3, 1])
        );
    }

    #[test]
    fn several_without_an_exact_match_are_ambiguous() {
        let results = vec![hit(0, MatchTier::Substring), hit(1, MatchTier::Substring)];
        assert_matches!(resolve(results), Resolution::Ambiguous(ref r) if r.len() == 2);
    }
}
