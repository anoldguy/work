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
