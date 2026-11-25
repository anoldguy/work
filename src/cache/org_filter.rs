//! Which orgs a cache update fetches (pure).

use crate::domain::Org;

/// Everything that decides one domain's org list. All comparisons are
/// ASCII case-insensitive.
#[derive(Debug, Clone, Copy)]
pub struct OrgSelection<'a> {
    /// Orgs the API says the user belongs to.
    pub member: &'a [Org],
    /// The domain's configured `extra_orgs`.
    pub extra: &'a [Org],
    /// Global `include_orgs`; when non-empty, only these orgs are kept.
    pub include: &'a [Org],
    /// Global `exclude_orgs`; ignored when `include` is non-empty.
    pub exclude: &'a [Org],
    /// The domain's `favorite_orgs`; when non-empty (and not `all_orgs`),
    /// only these orgs are kept.
    pub favorites: &'a [Org],
    /// `update --all-orgs`: ignore favourites.
    pub all_orgs: bool,
}

/// `member ∪ extra`, de-duplicated case-insensitively (first spelling wins,
/// member order first), then the global filters, then favourites unless
/// `all_orgs`.
#[must_use]
pub fn select_orgs(sel: &OrgSelection<'_>) -> Vec<Org> {
    let mut merged: Vec<Org> = Vec::with_capacity(sel.member.len() + sel.extra.len());
    for org in sel.member.iter().chain(sel.extra) {
        if !merged.iter().any(|seen| seen.eq_ignore_case(org)) {
            merged.push(org.clone());
        }
    }

    merged.retain(|org| passes_filters(org, sel.include, sel.exclude));

    if !sel.all_orgs && !sel.favorites.is_empty() {
        merged.retain(|org| sel.favorites.iter().any(|fav| fav.eq_ignore_case(org)));
    }

    merged
}

/// The global include/exclude rule for one owner: in `include` when that is
/// non-empty, otherwise not in `exclude`.
#[must_use]
pub fn passes_filters(owner: &Org, include: &[Org], exclude: &[Org]) -> bool {
    if include.is_empty() {
        !exclude.iter().any(|ex| ex.eq_ignore_case(owner))
    } else {
        include.iter().any(|inc| inc.eq_ignore_case(owner))
    }
}
