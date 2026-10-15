//! [`FakeGh`]: a scripted `Gh`.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::domain::{Domain, Org, Repo};
use crate::gh::{Gh, GhError};
use crate::proc::Interactive;

/// One recorded `auth` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhCall {
    /// `token`, `status`, `login`, or `logout`.
    pub action: &'static str,
    /// The host it was called for.
    pub host: Domain,
}

/// A `gh api` endpoint, for scripting failures and reading the call log.
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

/// One recorded API call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiCall {
    /// The domain.
    pub domain: Domain,
    /// The endpoint.
    pub endpoint: Endpoint,
}

/// A `Gh` scripted per host, recording every call.
///
/// Auth: `auth_status` succeeds exactly when the host has a scripted token
/// (one scripted as an error fails). A token or viewer registered "after
/// login" appears once `login` has been called for that host.
///
/// API: an unscripted viewer fails as gh does when not logged in; unscripted
/// orgs and listings are empty. A scripted failure returns
/// [`GhError::Api`] with the scripted message. Listings call `on_repo` once
/// per repo, and the fake tracks the most API calls in flight at once.
#[derive(Default)]
pub struct FakeGh {
    tokens: Mutex<BTreeMap<Domain, Result<String, GhError>>>,
    after_login: BTreeMap<Domain, String>,
    viewers: Mutex<BTreeMap<Domain, Org>>,
    viewers_after_login: BTreeMap<Domain, Org>,
    member_orgs: BTreeMap<Domain, Vec<Org>>,
    user_repos: BTreeMap<Domain, Vec<Repo>>,
    org_repos: BTreeMap<(Domain, Org), Vec<Repo>>,
    failures: BTreeMap<(Domain, Endpoint), String>,
    skipped: BTreeMap<(Domain, Endpoint), u32>,
    log: Mutex<Vec<GhCall>>,
    api_log: Mutex<Vec<ApiCall>>,
    /// (currently running, most ever running).
    in_flight: Mutex<(usize, usize)>,
}

impl std::fmt::Debug for FakeGh {
    /// Hosts and call counts only: the scripted tokens stay out of `Debug`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeGh")
            .field("hosts", &lock(&self.tokens).keys().collect::<Vec<_>>())
            .field("calls", &lock(&self.log).len())
            .field("api_calls", &lock(&self.api_log).len())
            .finish_non_exhaustive()
    }
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

impl FakeGh {
    /// A fake where no host has a token or any data.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `auth_status(host)` succeeds.
    #[must_use]
    pub fn with_token(self, host: Domain, token: &str) -> Self {
        lock(&self.tokens).insert(host, Ok(token.to_owned()));
        self
    }

    /// `auth_status(host)` fails with `err`.
    #[must_use]
    pub fn with_error(self, host: Domain, err: GhError) -> Self {
        lock(&self.tokens).insert(host, Err(err));
        self
    }

    /// `auth_status(host)` succeeds only after `login(host)`.
    #[must_use]
    pub fn with_token_after_login(mut self, host: Domain, token: &str) -> Self {
        self.after_login.insert(host, token.to_owned());
        self
    }

    /// `viewer(host)` returns `login`.
    #[must_use]
    pub fn with_viewer(self, host: Domain, login: Org) -> Self {
        lock(&self.viewers).insert(host, login);
        self
    }

    /// `viewer(host)` returns `login` only after `login(host)`.
    #[must_use]
    pub fn with_viewer_after_login(mut self, host: Domain, login: Org) -> Self {
        self.viewers_after_login.insert(host, login);
        self
    }

    /// `member_orgs(host)` returns `orgs`.
    #[must_use]
    pub fn with_member_orgs(mut self, host: Domain, orgs: Vec<Org>) -> Self {
        self.member_orgs.insert(host, orgs);
        self
    }

    /// `user_repos(host)` lists `repos`.
    #[must_use]
    pub fn with_user_repos(mut self, host: Domain, repos: Vec<Repo>) -> Self {
        self.user_repos.insert(host, repos);
        self
    }

    /// `org_repos(host, org)` lists `repos`.
    #[must_use]
    pub fn with_org_repos(mut self, host: Domain, org: Org, repos: Vec<Repo>) -> Self {
        self.org_repos.insert((host, org), repos);
        self
    }

    /// `endpoint` on `host` fails with gh's one-line `message`.
    #[must_use]
    pub fn with_failure(mut self, host: Domain, endpoint: Endpoint, message: &str) -> Self {
        self.failures.insert((host, endpoint), message.to_owned());
        self
    }

    /// The listing at `endpoint` on `host` reports `n` skipped lines.
    #[must_use]
    pub fn with_skipped(mut self, host: Domain, endpoint: Endpoint, n: u32) -> Self {
        self.skipped.insert((host, endpoint), n);
        self
    }

    /// Every `auth` call so far, in order.
    #[must_use]
    pub fn calls(&self) -> Vec<GhCall> {
        lock(&self.log).clone()
    }

    /// Every API call so far, in order.
    #[must_use]
    pub fn api_calls(&self) -> Vec<ApiCall> {
        lock(&self.api_log).clone()
    }

    /// The most API calls that were running at the same time.
    #[must_use]
    pub fn max_in_flight(&self) -> usize {
        lock(&self.in_flight).1
    }

    fn record(&self, action: &'static str, host: &Domain) {
        lock(&self.log).push(GhCall {
            action,
            host: host.clone(),
        });
    }

    /// Records the API call, then fails it if a failure is scripted.
    fn begin(&self, host: &Domain, endpoint: Endpoint) -> Result<(), GhError> {
        let key = (host.clone(), endpoint);
        let failure = self.failures.get(&key).cloned();
        let (domain, endpoint) = key;
        lock(&self.api_log).push(ApiCall { domain, endpoint });
        failure.map_or(Ok(()), |detail| {
            Err(GhError::Api {
                host: host.clone(),
                detail,
            })
        })
    }

    fn list(
        &self,
        host: &Domain,
        endpoint: Endpoint,
        repos: Option<&Vec<Repo>>,
        on_repo: &mut dyn FnMut(Repo),
    ) -> Result<u32, GhError> {
        let _guard = InFlight::enter(&self.in_flight);
        let skipped = self
            .skipped
            .get(&(host.clone(), endpoint.clone()))
            .copied()
            .unwrap_or(0);
        self.begin(host, endpoint)?;
        repos.into_iter().flatten().cloned().for_each(on_repo);
        Ok(skipped)
    }
}

impl Gh for FakeGh {
    fn viewer(&self, host: &Domain) -> Result<Org, GhError> {
        let _guard = InFlight::enter(&self.in_flight);
        self.begin(host, Endpoint::Viewer)?;
        lock(&self.viewers)
            .get(host)
            .cloned()
            .ok_or_else(|| GhError::Api {
                host: host.clone(),
                detail: format!(
                    "You are not logged into any GitHub hosts. To log in, run: gh auth login --hostname {host}"
                ),
            })
    }

    fn member_orgs(&self, host: &Domain) -> Result<Vec<Org>, GhError> {
        let _guard = InFlight::enter(&self.in_flight);
        self.begin(host, Endpoint::MemberOrgs)?;
        Ok(self.member_orgs.get(host).cloned().unwrap_or_default())
    }

    fn user_repos(&self, host: &Domain, on_repo: &mut dyn FnMut(Repo)) -> Result<u32, GhError> {
        self.list(
            host,
            Endpoint::UserRepos,
            self.user_repos.get(host),
            on_repo,
        )
    }

    fn org_repos(
        &self,
        host: &Domain,
        org: &Org,
        on_repo: &mut dyn FnMut(Repo),
    ) -> Result<u32, GhError> {
        let repos = self.org_repos.get(&(host.clone(), org.clone()));
        self.list(host, Endpoint::OrgRepos(org.clone()), repos, on_repo)
    }

    fn auth_status(&self, _term: &mut Interactive<'_>, host: &Domain) -> Result<(), GhError> {
        self.record("status", host);
        match lock(&self.tokens).get(host) {
            Some(Ok(_)) => Ok(()),
            Some(Err(err)) => Err(err.clone()),
            None => Err(GhError::Failed {
                action: "status",
                host: host.clone(),
                detail: "exit status: 1".to_owned(),
            }),
        }
    }

    fn login(&self, _term: &mut Interactive<'_>, host: &Domain) -> Result<(), GhError> {
        self.record("login", host);
        if let Some(token) = self.after_login.get(host) {
            lock(&self.tokens).insert(host.clone(), Ok(token.clone()));
        }
        if let Some(login) = self.viewers_after_login.get(host) {
            lock(&self.viewers).insert(host.clone(), login.clone());
        }
        Ok(())
    }

    fn logout(&self, _term: &mut Interactive<'_>, host: &Domain) -> Result<(), GhError> {
        self.record("logout", host);
        lock(&self.tokens).remove(host);
        lock(&self.viewers).remove(host);
        Ok(())
    }
}

/// Lets a test keep a handle on the `FakeGh` it boxes into an `App`.
impl Gh for std::sync::Arc<FakeGh> {
    fn viewer(&self, host: &Domain) -> Result<Org, GhError> {
        (**self).viewer(host)
    }

    fn member_orgs(&self, host: &Domain) -> Result<Vec<Org>, GhError> {
        (**self).member_orgs(host)
    }

    fn user_repos(&self, host: &Domain, on_repo: &mut dyn FnMut(Repo)) -> Result<u32, GhError> {
        (**self).user_repos(host, on_repo)
    }

    fn org_repos(
        &self,
        host: &Domain,
        org: &Org,
        on_repo: &mut dyn FnMut(Repo),
    ) -> Result<u32, GhError> {
        (**self).org_repos(host, org, on_repo)
    }

    fn auth_status(&self, term: &mut Interactive<'_>, host: &Domain) -> Result<(), GhError> {
        (**self).auth_status(term, host)
    }

    fn login(&self, term: &mut Interactive<'_>, host: &Domain) -> Result<(), GhError> {
        (**self).login(term, host)
    }

    fn logout(&self, term: &mut Interactive<'_>, host: &Domain) -> Result<(), GhError> {
        (**self).logout(term, host)
    }
}
