//! [`FakeGitHub`] (filled in by W2.8) and [`NullGitHub`].

use std::sync::Mutex;

use crate::auth::SecretToken;
use crate::domain::{Domain, Org, Repo};
use crate::github::{ApiError, GitHubApi, Listing, Page};

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
    in_flight: Mutex<(usize, usize)>,
}

impl FakeGitHub {
    /// A fake with no domains.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `viewer(d)` returns `login`.
    #[must_use]
    pub fn with_viewer(self, d: Domain, login: Org) -> Self {
        let _ = (self, d, login);
        unimplemented!("W2.8")
    }

    /// `member_orgs(d)` returns `orgs`.
    #[must_use]
    pub fn with_member_orgs(self, d: Domain, orgs: Vec<Org>) -> Self {
        let _ = (self, d, orgs);
        unimplemented!("W2.8")
    }

    /// `user_repos(d)` serves these pages.
    #[must_use]
    pub fn with_user_repos(self, d: Domain, pages: Vec<Vec<Repo>>) -> Self {
        let _ = (self, d, pages);
        unimplemented!("W2.8")
    }

    /// `org_repos(d, org)` serves these pages.
    #[must_use]
    pub fn with_org_repos(self, d: Domain, org: Org, pages: Vec<Vec<Repo>>) -> Self {
        let _ = (self, d, org, pages);
        unimplemented!("W2.8")
    }

    /// `endpoint` on `d` fails with HTTP `status`.
    #[must_use]
    pub fn with_status(self, d: Domain, endpoint: Endpoint, status: u16) -> Self {
        let _ = (self, d, endpoint, status);
        unimplemented!("W2.8")
    }

    /// Every call so far, in order.
    #[must_use]
    pub fn calls(&self) -> Vec<ApiCall> {
        let _ = &self.log;
        unimplemented!("W2.8")
    }

    /// The most calls that were running at the same time.
    #[must_use]
    pub fn max_in_flight(&self) -> usize {
        let _ = &self.in_flight;
        unimplemented!("W2.8")
    }
}

impl GitHubApi for FakeGitHub {
    fn viewer(&self, d: &Domain, t: &SecretToken) -> Result<Org, ApiError> {
        let _ = (self, d, t);
        unimplemented!("W2.8")
    }

    fn member_orgs(&self, d: &Domain, t: &SecretToken) -> Result<Vec<Org>, ApiError> {
        let _ = (self, d, t);
        unimplemented!("W2.8")
    }

    fn user_repos(
        &self,
        d: &Domain,
        t: &SecretToken,
        on_page: &mut dyn FnMut(Page),
    ) -> Result<Listing, ApiError> {
        let _ = (self, d, t, on_page);
        unimplemented!("W2.8")
    }

    fn org_repos(
        &self,
        d: &Domain,
        t: &SecretToken,
        org: &Org,
        on_page: &mut dyn FnMut(Page),
    ) -> Result<Listing, ApiError> {
        let _ = (self, d, t, org, on_page);
        unimplemented!("W2.8")
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
