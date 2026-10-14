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
    let _ = sel;
    unimplemented!("W2.3")
}

/// The global include/exclude rule for one owner: in `include` when that is
/// non-empty, otherwise not in `exclude`. Used for collaborator repos in the
/// personal job.
#[must_use]
pub fn passes_filters(owner: &Org, include: &[Org], exclude: &[Org]) -> bool {
    let _ = (owner, include, exclude);
    unimplemented!("W2.3")
}
