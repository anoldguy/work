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
/// non-empty, otherwise not in `exclude`. Used for collaborator repos in the
/// personal job.
#[must_use]
pub fn passes_filters(owner: &Org, include: &[Org], exclude: &[Org]) -> bool {
    if include.is_empty() {
        !exclude.iter().any(|ex| ex.eq_ignore_case(owner))
    } else {
        include.iter().any(|inc| inc.eq_ignore_case(owner))
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::{OrgSelection, passes_filters, select_orgs};
    use crate::domain::Org;

    fn orgs(names: &[&str]) -> Vec<Org> {
        names
            .iter()
            .map(|n| n.parse().expect("test org is valid"))
            .collect()
    }

    fn names(list: &[Org]) -> Vec<&str> {
        list.iter().map(Org::as_str).collect()
    }

    fn select(
        member: &[Org],
        extra: &[Org],
        include: &[Org],
        exclude: &[Org],
        favorites: &[Org],
        all_orgs: bool,
    ) -> Vec<Org> {
        select_orgs(&OrgSelection {
            member,
            extra,
            include,
            exclude,
            favorites,
            all_orgs,
        })
    }

    #[test]
    fn member_and_extra_orgs_are_merged_and_deduplicated_case_insensitively() {
        let member = orgs(&["Acme", "beta"]);
        let extra = orgs(&["ACME", "partner"]);
        let out = select(&member, &extra, &[], &[], &[], false);
        assert_matches!(names(&out).as_slice(), ["Acme", "beta", "partner"]);
    }

    #[test]
    fn include_list_keeps_only_those_orgs() {
        let member = orgs(&["acme", "beta", "gamma"]);
        let include = orgs(&["BETA"]);
        let out = select(&member, &[], &include, &[], &[], false);
        assert_matches!(names(&out).as_slice(), ["beta"]);
    }

    #[test]
    fn include_takes_precedence_over_exclude() {
        let member = orgs(&["acme", "beta"]);
        let include = orgs(&["acme"]);
        let exclude = orgs(&["acme"]);
        let out = select(&member, &[], &include, &exclude, &[], false);
        assert_matches!(names(&out).as_slice(), ["acme"]);
    }

    #[test]
    fn exclude_removes_orgs_case_insensitively_when_no_include() {
        let member = orgs(&["acme", "beta"]);
        let exclude = orgs(&["ACME"]);
        let out = select(&member, &[], &[], &exclude, &[], false);
        assert_matches!(names(&out).as_slice(), ["beta"]);
    }

    #[test]
    fn favorites_narrow_the_list_unless_all_orgs() {
        let member = orgs(&["acme", "beta", "gamma"]);
        let favorites = orgs(&["GAMMA"]);
        let narrowed = select(&member, &[], &[], &[], &favorites, false);
        assert_matches!(names(&narrowed).as_slice(), ["gamma"]);

        let everything = select(&member, &[], &[], &[], &favorites, true);
        assert_matches!(names(&everything).as_slice(), ["acme", "beta", "gamma"]);
    }

    #[test]
    fn no_favorites_keeps_every_org_after_global_filters() {
        let member = orgs(&["acme", "beta"]);
        let out = select(&member, &[], &[], &[], &[], false);
        assert_matches!(names(&out).as_slice(), ["acme", "beta"]);
    }

    #[test]
    fn extra_org_that_is_favorite_is_kept_even_when_not_a_member() {
        let member = orgs(&["acme"]);
        let extra = orgs(&["partner"]);
        let favorites = orgs(&["partner"]);
        let out = select(&member, &extra, &[], &[], &favorites, false);
        assert_matches!(names(&out).as_slice(), ["partner"]);
    }

    #[test]
    fn passes_filters_applies_include_then_exclude() {
        let acme = orgs(&["Acme"]).remove(0);
        assert!(passes_filters(&acme, &[], &[]));
        assert!(passes_filters(&acme, &orgs(&["acme"]), &[]));
        assert!(!passes_filters(&acme, &orgs(&["beta"]), &[]));
        assert!(!passes_filters(&acme, &[], &orgs(&["ACME"])));
        assert_matches!(
            passes_filters(&acme, &orgs(&["acme"]), &orgs(&["acme"])),
            true
        );
    }
}
