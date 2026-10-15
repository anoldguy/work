//! Folding fetch results into the previous cache (pure, infallible).

use crate::cache::{CACHE_VERSION, Cache};
use crate::config::Filters;
use crate::core::org_filter::passes_filters;
use crate::domain::{Domain, Org, Repo, StoredTime};

/// The result of fetching one org's repos (or a domain's personal repos).
#[derive(Debug)]
pub enum OrgOutcome {
    /// The repos fetched.
    Fetched(Vec<Repo>),
    /// Why it failed (one line). Previous repos are carried forward.
    Failed(String),
}

/// The result of refreshing one domain.
#[derive(Debug)]
pub enum DomainOutcome {
    /// The domain could not be refreshed at all (no token, `viewer` or org
    /// listing failed). All its previous repos are carried forward.
    Failed {
        /// The domain.
        domain: Domain,
        /// Why (one line).
        cause: String,
    },
    /// The org phase ran.
    Fetched {
        /// The domain.
        domain: Domain,
        /// The authenticated login.
        viewer: Org,
        /// The personal job: owner and collaborator repos.
        personal: OrgOutcome,
        /// Every selected org and its outcome. Orgs not listed here are
        /// filtered out and dropped from the cache.
        orgs: Vec<(Org, OrgOutcome)>,
        /// Member and extra orgs that were not selected. When the personal
        /// job fails, their previous repos are not carried forward (nor are
        /// those of owners the global filters reject; see [`merge`]). The
        /// viewer's own repos always are.
        deselected: Vec<Org>,
    },
}

/// True when `outcomes` is empty, or every domain is `Failed` or `Fetched`
/// with the personal job and every org failed. Refresh writes nothing then.
#[must_use]
pub fn all_failed(outcomes: &[DomainOutcome]) -> bool {
    outcomes.iter().all(|outcome| match outcome {
        DomainOutcome::Failed { .. } => true,
        DomainOutcome::Fetched { personal, orgs, .. } => {
            personal.is_failed() && orgs.iter().all(|(_, o)| o.is_failed())
        }
    })
}

impl OrgOutcome {
    const fn is_failed(&self) -> bool {
        matches!(self, Self::Failed(_))
    }
}

/// Which previous repos of one domain survive this run.
enum Carry {
    /// The domain failed: every previous repo survives.
    All,
    /// The org phase ran.
    Partial {
        /// Orgs whose fetch failed: their previous repos survive.
        failed: Vec<Org>,
        /// Orgs fetched successfully.
        fetched: Vec<Org>,
        /// The personal job failed: previous repos whose owner is the
        /// viewer, or is in neither `fetched` nor `deselected` and passes
        /// the global filters, survive.
        personal_failed: bool,
        /// The authenticated login.
        viewer: Org,
        /// Owners filtered out in this run.
        deselected: Vec<Org>,
    },
}

impl Carry {
    fn keeps(&self, owner: &Org, filters: &Filters) -> bool {
        match self {
            Self::All => true,
            Self::Partial {
                failed,
                fetched,
                personal_failed,
                viewer,
                deselected,
            } => {
                let listed = |list: &[Org]| list.iter().any(|o| o.eq_ignore_case(owner));
                // Only the failed personal job could have listed this owner.
                let personal_only = || {
                    !listed(fetched)
                        && !listed(deselected)
                        && passes_filters(owner, &filters.include_orgs, &filters.exclude_orgs)
                };
                listed(failed)
                    || (*personal_failed && (owner.eq_ignore_case(viewer) || personal_only()))
            }
        }
    }
}

/// Builds the new cache.
///
/// - A failed org carries forward its previous `(domain, org)` repos.
/// - A failed personal job carries forward the domain's previous repos whose
///   owner is the viewer, or is neither an org fetched successfully in this
///   run nor a `deselected` owner, and passes the global `filters` (so a
///   collaborator repo outside `include_orgs` or in `exclude_orgs` is not
///   kept).
/// - A failed domain carries forward all its previous repos.
/// - Domains not in `configured` and orgs no longer selected are dropped.
/// - Repos are de-duplicated by slug; the org-phase copy wins.
/// - A configured domain with no outcome keeps its previous repos.
/// - `updated_at` is `now` unless [`all_failed`] holds for `outcomes`; then
///   the previous timestamp is kept (refresh writes nothing in that case).
#[must_use]
pub fn merge(
    prev: Option<Cache>,
    outcomes: Vec<DomainOutcome>,
    configured: &[Domain],
    filters: &Filters,
    now: StoredTime,
) -> Cache {
    let success = !all_failed(&outcomes);
    let is_configured = |d: &Domain| configured.contains(d);

    // Fetched repos in priority order: org phase first, then personal.
    let mut org_repos = Vec::new();
    let mut personal_repos = Vec::new();
    let mut rules: Vec<(Domain, Carry)> = Vec::with_capacity(outcomes.len());
    for outcome in outcomes {
        match outcome {
            DomainOutcome::Failed { domain, .. } => {
                if is_configured(&domain) {
                    rules.push((domain, Carry::All));
                }
            }
            DomainOutcome::Fetched {
                domain,
                viewer,
                personal,
                orgs,
                deselected,
            } => {
                if !is_configured(&domain) {
                    continue;
                }
                let mut failed = Vec::new();
                let mut fetched = Vec::new();
                for (org, result) in orgs {
                    match result {
                        OrgOutcome::Fetched(repos) => {
                            org_repos.extend(repos);
                            fetched.push(org);
                        }
                        OrgOutcome::Failed(_) => failed.push(org),
                    }
                }
                let personal_failed = match personal {
                    OrgOutcome::Fetched(repos) => {
                        personal_repos.extend(repos);
                        false
                    }
                    OrgOutcome::Failed(_) => true,
                };
                rules.push((
                    domain,
                    Carry::Partial {
                        failed,
                        fetched,
                        personal_failed,
                        viewer,
                        deselected,
                    },
                ));
            }
        }
    }

    let (prev_repos, prev_updated_at) = match prev {
        Some(Cache {
            repos, updated_at, ..
        }) => (repos, Some(updated_at)),
        None => (Vec::new(), None),
    };

    let carried = prev_repos.into_iter().filter(|repo| {
        let domain = repo.slug.domain();
        is_configured(domain)
            && rules
                .iter()
                .find(|(d, _)| d == domain)
                .is_none_or(|(_, carry)| carry.keeps(repo.slug.org(), filters))
    });

    // Stable sort keeps the priority order among equal slugs, so `dedup_by`
    // keeps the org-phase copy, then the personal one, then the carried one.
    let mut repos: Vec<Repo> = org_repos
        .into_iter()
        .chain(personal_repos)
        .chain(carried)
        .collect();
    repos.sort_by(|a, b| a.slug.cmp(&b.slug));
    repos.dedup_by(|later, earlier| later.slug == earlier.slug);

    let updated_at = match prev_updated_at {
        Some(previous) if !success => previous,
        _ => now,
    };
    Cache {
        version: CACHE_VERSION,
        updated_at,
        repos,
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::domain::{CloneUrl, HtmlUrl, Slug};

    const NOW: i64 = 2_000_000;
    const BEFORE: i64 = 1_000_000;

    fn t(secs: i64) -> StoredTime {
        StoredTime::from_unix_seconds(secs).unwrap()
    }

    fn dom(d: &str) -> Domain {
        Domain::try_from(d.to_owned()).unwrap()
    }

    fn org(o: &str) -> Org {
        Org::try_from(o.to_owned()).unwrap()
    }

    fn slug(s: &str) -> Slug {
        Slug::try_from(s.to_owned()).unwrap()
    }

    /// `s` is `{domain}/{org}/{repo}`; `tag` marks which copy it is.
    fn repo(s: &str, tag: &str) -> Repo {
        Repo {
            slug: slug(s),
            clone_url: CloneUrl::try_from(format!("https://{s}.git")).unwrap(),
            html_url: HtmlUrl::try_from(format!("https://{s}?{tag}")).unwrap(),
        }
    }

    fn fetched(slugs: &[&str]) -> OrgOutcome {
        OrgOutcome::Fetched(slugs.iter().map(|s| repo(s, "new")).collect())
    }

    fn failed() -> OrgOutcome {
        OrgOutcome::Failed("boom".to_owned())
    }

    fn domain_ok(d: &str, personal: OrgOutcome, orgs: Vec<(&str, OrgOutcome)>) -> DomainOutcome {
        domain_ok_without(d, personal, orgs, &[])
    }

    fn domain_ok_without(
        d: &str,
        personal: OrgOutcome,
        orgs: Vec<(&str, OrgOutcome)>,
        deselected: &[&str],
    ) -> DomainOutcome {
        DomainOutcome::Fetched {
            domain: dom(d),
            viewer: org("me"),
            personal,
            orgs: orgs.into_iter().map(|(o, r)| (org(o), r)).collect(),
            deselected: deselected.iter().map(|o| org(o)).collect(),
        }
    }

    fn domain_failed(d: &str) -> DomainOutcome {
        DomainOutcome::Failed {
            domain: dom(d),
            cause: "no token".to_owned(),
        }
    }

    /// A previous cache holding `slugs`.
    fn prev(slugs: &[&str]) -> Cache {
        Cache::new(slugs.iter().map(|s| repo(s, "old")).collect(), t(BEFORE))
    }

    fn slugs(c: &Cache) -> Vec<String> {
        c.repos.iter().map(|r| r.slug.to_string()).collect()
    }

    fn gh() -> Vec<Domain> {
        vec![dom("github.com")]
    }

    #[test]
    fn all_failed_when_personal_and_every_org_failed() {
        let o = [
            domain_failed("ghe.example"),
            domain_ok(
                "github.com",
                failed(),
                vec![("acme", failed()), ("beta", failed())],
            ),
        ];
        assert!(all_failed(&o));
    }

    #[test]
    fn failed_org_keeps_its_previous_repos() {
        let p = prev(&[
            "github.com/acme/a",
            "github.com/beta/b",
            "github.com/beta/gone",
        ]);
        let c = merge(
            Some(p),
            vec![domain_ok(
                "github.com",
                fetched(&[]),
                vec![
                    ("ACME", failed()),
                    ("beta", fetched(&["github.com/beta/b"])),
                ],
            )],
            &gh(),
            &Filters::default(),
            t(NOW),
        );
        assert_eq!(slugs(&c), ["github.com/acme/a", "github.com/beta/b"]);
        assert_eq!(
            c.repos[0].html_url.as_str(),
            "https://github.com/acme/a?old"
        );
        assert_eq!(
            c.repos[1].html_url.as_str(),
            "https://github.com/beta/b?new"
        );
        assert_eq!(c.updated_at, t(NOW));
    }

    #[test]
    fn failed_personal_keeps_repos_not_owned_by_fetched_orgs() {
        let p = prev(&[
            "github.com/me/dots",
            "github.com/friend/shared",
            "github.com/acme/gone",
        ]);
        let c = merge(
            Some(p),
            vec![domain_ok(
                "github.com",
                failed(),
                vec![("acme", fetched(&["github.com/acme/a"]))],
            )],
            &gh(),
            &Filters::default(),
            t(NOW),
        );
        assert_eq!(
            slugs(&c),
            [
                "github.com/acme/a",
                "github.com/friend/shared",
                "github.com/me/dots"
            ]
        );
    }

    #[test]
    fn successful_personal_drops_vanished_personal_repos() {
        let p = prev(&["github.com/me/dots", "github.com/me/old"]);
        let c = merge(
            Some(p),
            vec![domain_ok(
                "github.com",
                fetched(&["github.com/me/dots"]),
                vec![],
            )],
            &gh(),
            &Filters::default(),
            t(NOW),
        );
        assert_eq!(slugs(&c), ["github.com/me/dots"]);
    }

    #[test]
    fn failed_domain_keeps_all_its_previous_repos() {
        let p = prev(&[
            "ghe.example/acme/a",
            "ghe.example/me/b",
            "github.com/acme/old",
        ]);
        let c = merge(
            Some(p),
            vec![
                domain_failed("ghe.example"),
                domain_ok("github.com", fetched(&["github.com/me/x"]), vec![]),
            ],
            &[dom("github.com"), dom("ghe.example")],
            &Filters::default(),
            t(NOW),
        );
        assert_eq!(
            slugs(&c),
            ["ghe.example/acme/a", "ghe.example/me/b", "github.com/me/x"]
        );
    }

    #[test]
    fn unconfigured_domain_is_dropped() {
        let p = prev(&["ghe.example/acme/a", "github.com/acme/a"]);
        let c = merge(
            Some(p),
            vec![
                domain_ok("github.com", failed(), vec![("acme", failed())]),
                domain_failed("ghe.example"),
                domain_ok("old.example", fetched(&["old.example/me/x"]), vec![]),
            ],
            &gh(),
            &Filters::default(),
            t(NOW),
        );
        assert_eq!(slugs(&c), ["github.com/acme/a"]);
    }

    #[test]
    fn deselected_org_is_dropped() {
        let p = prev(&["github.com/acme/a", "github.com/beta/b"]);
        let c = merge(
            Some(p),
            vec![domain_ok(
                "github.com",
                fetched(&[]),
                vec![("acme", fetched(&["github.com/acme/a"]))],
            )],
            &gh(),
            &Filters::default(),
            t(NOW),
        );
        assert_eq!(slugs(&c), ["github.com/acme/a"]);
    }

    #[test]
    fn dedup_prefers_org_phase_copy() {
        let o = vec![domain_ok(
            "github.com",
            OrgOutcome::Fetched(vec![repo("github.com/acme/a", "personal")]),
            vec![(
                "acme",
                OrgOutcome::Fetched(vec![
                    repo("github.com/acme/a", "org"),
                    repo("github.com/acme/a", "org-dup"),
                ]),
            )],
        )];
        let c = merge(
            Some(prev(&["github.com/acme/a"])),
            o,
            &gh(),
            &Filters::default(),
            t(NOW),
        );
        assert_matches!(
            &c.repos[..],
            [r] if r.html_url.as_str() == "https://github.com/acme/a?org"
        );
    }

    #[test]
    fn total_failure_keeps_previous_timestamp_and_repos() {
        let c = merge(
            Some(prev(&["github.com/acme/a"])),
            vec![domain_failed("github.com")],
            &gh(),
            &Filters::default(),
            t(NOW),
        );
        assert_eq!(c.updated_at, t(BEFORE));
        assert_eq!(slugs(&c), ["github.com/acme/a"]);
    }
}
