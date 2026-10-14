//! Turns ranked search results into zero, one, or an ambiguous set (pure).

use crate::core::search::SearchResult;

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
    drop(results);
    unimplemented!("W2.2")
}
