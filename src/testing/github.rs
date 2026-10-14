//! [`FakeGitHub`] and [`NullGitHub`].

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::auth::SecretToken;
use crate::domain::{Domain, Org, Repo};
use crate::github::{ApiError, GitHubApi, Listing, Page, StatusHint};

/// An API endpoint, for scripting failures and reading the call log.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Endpoint {
    /// `viewer`.
    Viewer,
    /// `member_orgs`.
    MemberOrgs,
    /// `user_repos`.
    UserRepos,
    /// `org_repos` for this org.
    OrgRepos(Org),
}

impl Endpoint {
    /// The REST path the real client would request.
    fn path(&self) -> String {
        match self {
            Self::Viewer => "/user".to_owned(),
            Self::MemberOrgs => "/user/orgs".to_owned(),
            Self::UserRepos => "/user/repos".to_owned(),
            Self::OrgRepos(org) => format!("/orgs/{org}/repos"),
        }
    }
}

/// One recorded call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiCall {
    /// The domain.
    pub domain: Domain,
    /// The endpoint.
    pub endpoint: Endpoint,
}

/// A `GitHubApi` that serves scripted pages, calls `on_page` once per page,
/// records every call, and tracks the most calls in flight at once (for the
/// "≤ 8 across domains" test).
///
/// An unscripted domain fails `viewer` with HTTP 404; an unscripted org has
/// no repos. A scripted failure returns `ApiError::Status` with that status.
///
/// The scripted data's private storage is W2.8's choice; only the builder
/// and inspection methods are contract.
#[derive(Debug, Default)]
pub struct FakeGitHub {
    log: Mutex<Vec<ApiCall>>,
    /// (currently running, most ever running).
    in_flight: Mutex<(usize, usize)>,
    viewers: BTreeMap<Domain, Org>,
    member_orgs: BTreeMap<Domain, Vec<Org>>,
    user_repos: BTreeMap<Domain, Vec<Vec<Repo>>>,
    org_repos: BTreeMap<(Domain, Org), Vec<Vec<Repo>>>,
    statuses: BTreeMap<(Domain, Endpoint), u16>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Marks one call in flight until dropped.
struct InFlight<'a>(&'a Mutex<(usize, usize)>);

impl<'a> InFlight<'a> {
    fn enter(counter: &'a Mutex<(usize, usize)>) -> Self {
        let mut c = lock(counter);
        c.0 += 1;
        c.1 = c.1.max(c.0);
        drop(c);
        Self(counter)
    }
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        lock(self.0).0 -= 1;
    }
}

impl FakeGitHub {
    /// A fake with no domains.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `viewer(d)` returns `login`.
    #[must_use]
    pub fn with_viewer(mut self, d: Domain, login: Org) -> Self {
        self.viewers.insert(d, login);
        self
    }

    /// `member_orgs(d)` returns `orgs`.
    #[must_use]
    pub fn with_member_orgs(mut self, d: Domain, orgs: Vec<Org>) -> Self {
        self.member_orgs.insert(d, orgs);
        self
    }

    /// `user_repos(d)` serves these pages.
    #[must_use]
    pub fn with_user_repos(mut self, d: Domain, pages: Vec<Vec<Repo>>) -> Self {
        self.user_repos.insert(d, pages);
        self
    }

    /// `org_repos(d, org)` serves these pages.
    #[must_use]
    pub fn with_org_repos(mut self, d: Domain, org: Org, pages: Vec<Vec<Repo>>) -> Self {
        self.org_repos.insert((d, org), pages);
        self
    }

    /// `endpoint` on `d` fails with HTTP `status`.
    #[must_use]
    pub fn with_status(mut self, d: Domain, endpoint: Endpoint, status: u16) -> Self {
        self.statuses.insert((d, endpoint), status);
        self
    }

    /// Every call so far, in order.
    #[must_use]
    pub fn calls(&self) -> Vec<ApiCall> {
        lock(&self.log).clone()
    }

    /// The most calls that were running at the same time.
    #[must_use]
    pub fn max_in_flight(&self) -> usize {
        lock(&self.in_flight).1
    }

    /// Records the call, then fails it if a status is scripted.
    fn begin(&self, d: &Domain, endpoint: Endpoint) -> Result<(), ApiError> {
        let key = (d.clone(), endpoint);
        let status = self.statuses.get(&key).copied();
        let (domain, endpoint) = key;
        let err = status.map(|status| status_error(status, &domain, &endpoint));
        lock(&self.log).push(ApiCall { domain, endpoint });
        err.map_or(Ok(()), Err)
    }

    /// Serves `pages`, calling `on_page` after each.
    fn serve(pages: Option<&Vec<Vec<Repo>>>, on_page: &mut dyn FnMut(Page)) -> Listing {
        let mut listing = Listing::default();
        for (number, page) in (1..).zip(pages.into_iter().flatten()) {
            listing.repos.extend(page.iter().cloned());
            on_page(Page {
                number,
                repos_so_far: u32::try_from(listing.repos.len()).unwrap_or(u32::MAX),
            });
        }
        listing
    }
}

fn status_error(status: u16, d: &Domain, endpoint: &Endpoint) -> ApiError {
    let hint = match status {
        401 => StatusHint::Login { domain: d.clone() },
        429 => StatusHint::RateLimit,
        403 => StatusHint::Forbidden,
        404 if matches!(endpoint, Endpoint::OrgRepos(_)) => StatusHint::OrgNotFound,
        500..=599 => StatusHint::ServerError,
        _ => StatusHint::None,
    };
    ApiError::Status {
        status,
        path: endpoint.path(),
        hint,
    }
}

impl GitHubApi for FakeGitHub {
    fn viewer(&self, d: &Domain, _t: &SecretToken) -> Result<Org, ApiError> {
        let _guard = InFlight::enter(&self.in_flight);
        self.begin(d, Endpoint::Viewer)?;
        self.viewers
            .get(d)
            .cloned()
            .ok_or_else(|| status_error(404, d, &Endpoint::Viewer))
    }

    fn member_orgs(&self, d: &Domain, _t: &SecretToken) -> Result<Vec<Org>, ApiError> {
        let _guard = InFlight::enter(&self.in_flight);
        self.begin(d, Endpoint::MemberOrgs)?;
        Ok(self.member_orgs.get(d).cloned().unwrap_or_default())
    }

    fn user_repos(
        &self,
        d: &Domain,
        _t: &SecretToken,
        on_page: &mut dyn FnMut(Page),
    ) -> Result<Listing, ApiError> {
        let _guard = InFlight::enter(&self.in_flight);
        self.begin(d, Endpoint::UserRepos)?;
        Ok(Self::serve(self.user_repos.get(d), on_page))
    }

    fn org_repos(
        &self,
        d: &Domain,
        _t: &SecretToken,
        org: &Org,
        on_page: &mut dyn FnMut(Page),
    ) -> Result<Listing, ApiError> {
        let _guard = InFlight::enter(&self.in_flight);
        self.begin(d, Endpoint::OrgRepos(org.clone()))?;
        let key = (d.clone(), org.clone());
        Ok(Self::serve(self.org_repos.get(&key), on_page))
    }
}

/// A `GitHubApi` for tests that must never touch the API (for example the
/// shared pipeline in W3.3). Every method panics.
#[derive(Debug, Default)]
pub struct NullGitHub;

impl GitHubApi for NullGitHub {
    fn viewer(&self, d: &Domain, t: &SecretToken) -> Result<Org, ApiError> {
        unreachable!("NullGitHub::viewer({d}, {t:?}) was called")
    }

    fn member_orgs(&self, d: &Domain, t: &SecretToken) -> Result<Vec<Org>, ApiError> {
        unreachable!("NullGitHub::member_orgs({d}, {t:?}) was called")
    }

    fn user_repos(
        &self,
        d: &Domain,
        t: &SecretToken,
        _on_page: &mut dyn FnMut(Page),
    ) -> Result<Listing, ApiError> {
        unreachable!("NullGitHub::user_repos({d}, {t:?}) was called")
    }

    fn org_repos(
        &self,
        d: &Domain,
        t: &SecretToken,
        org: &Org,
        _on_page: &mut dyn FnMut(Page),
    ) -> Result<Listing, ApiError> {
        unreachable!("NullGitHub::org_repos({d}, {t:?}, {org}) was called")
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::{ApiCall, Endpoint, FakeGitHub};
    use crate::auth::SecretToken;
    use crate::domain::{Domain, Org, Repo};
    use crate::github::{ApiError, GitHubApi, Page, StatusHint};

    fn dom(s: &str) -> Domain {
        s.parse().unwrap()
    }

    fn org(s: &str) -> Org {
        s.parse().unwrap()
    }

    fn repo(slug: &str) -> Repo {
        let (_, path) = slug.split_once('/').unwrap();
        serde_json::from_value(serde_json::json!({
            "slug": slug,
            "clone_url": format!("https://github.com/{path}.git"),
            "html_url": format!("https://github.com/{path}"),
        }))
        .unwrap()
    }

    fn tok() -> SecretToken {
        SecretToken::new("tok").unwrap()
    }

    #[test]
    fn serves_scripted_data_and_records_calls() {
        let d = dom("github.com");
        let fake = FakeGitHub::new()
            .with_viewer(d.clone(), org("me"))
            .with_member_orgs(d.clone(), vec![org("acme")])
            .with_user_repos(d.clone(), vec![vec![repo("github.com/me/a")]])
            .with_org_repos(
                d.clone(),
                org("acme"),
                vec![
                    vec![repo("github.com/acme/x"), repo("github.com/acme/y")],
                    vec![repo("github.com/acme/z")],
                ],
            );
        assert_eq!(fake.viewer(&d, &tok()).unwrap(), org("me"));
        assert_eq!(fake.member_orgs(&d, &tok()).unwrap(), vec![org("acme")]);
        assert_eq!(
            fake.user_repos(&d, &tok(), &mut |_| {})
                .unwrap()
                .repos
                .len(),
            1
        );
        let mut pages = Vec::new();
        let listing = fake
            .org_repos(&d, &tok(), &org("acme"), &mut |p| pages.push(p))
            .unwrap();
        assert_eq!(listing.repos.len(), 3);
        assert_eq!(
            pages,
            vec![
                Page {
                    number: 1,
                    repos_so_far: 2
                },
                Page {
                    number: 2,
                    repos_so_far: 3
                }
            ]
        );
        let endpoints: Vec<Endpoint> = fake.calls().into_iter().map(|c| c.endpoint).collect();
        assert_eq!(
            endpoints,
            vec![
                Endpoint::Viewer,
                Endpoint::MemberOrgs,
                Endpoint::UserRepos,
                Endpoint::OrgRepos(org("acme"))
            ]
        );
        assert_eq!(fake.max_in_flight(), 1);
    }

    #[test]
    fn unscripted_domain_404s_and_unscripted_org_is_empty() {
        let fake = FakeGitHub::new();
        let d = dom("ghe.test");
        assert_matches!(
            fake.viewer(&d, &tok()),
            Err(ApiError::Status { status: 404, .. })
        );
        let listing = fake
            .org_repos(&d, &tok(), &org("x"), &mut |_| panic!("no pages"))
            .unwrap();
        assert_eq!(listing.repos, Vec::new());
    }

    #[test]
    fn scripted_status_fails_that_endpoint_and_is_logged() {
        let d = dom("github.com");
        let fake = FakeGitHub::new()
            .with_viewer(d.clone(), org("me"))
            .with_status(d.clone(), Endpoint::Viewer, 401)
            .with_status(d.clone(), Endpoint::OrgRepos(org("acme")), 502);
        assert_matches!(
            fake.viewer(&d, &tok()),
            Err(ApiError::Status {
                status: 401,
                hint: StatusHint::Login { .. },
                ..
            })
        );
        let err = fake
            .org_repos(&d, &tok(), &org("acme"), &mut |_| {})
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "HTTP 502 from /orgs/acme/repos (GitHub server error; try again later)"
        );
        assert_eq!(
            fake.calls()[1],
            ApiCall {
                domain: d,
                endpoint: Endpoint::OrgRepos(org("acme"))
            }
        );
    }
}
