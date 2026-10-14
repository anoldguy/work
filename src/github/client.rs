//! [`HttpGitHub`]: the blocking ureq client.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use crate::auth::SecretToken;
use crate::domain::{ApiUrl, Domain, Org};
use crate::github::{ApiError, GitHubApi, Listing, Page};

/// The real API client.
///
/// Sends `Accept: application/vnd.github+json`,
/// `X-GitHub-Api-Version: 2022-11-28` and `User-Agent: work/<version>`; never
/// forwards auth headers across redirects; follows `rel="next"` only within
/// the API base's origin, for at most `MAX_PAGES` pages.
#[derive(Debug)]
pub struct HttpGitHub {
    /// Built on first request (by W2.8's private `agent()`), so constructing
    /// the client costs nothing for commands that never call the API.
    agent: OnceLock<ureq::Agent>,
    bases: BTreeMap<Domain, ApiUrl>,
}

impl HttpGitHub {
    /// A client using `bases` for configured domains and the computed
    /// default (`api_base`) for any other domain.
    #[must_use]
    pub const fn new(bases: BTreeMap<Domain, ApiUrl>) -> Self {
        Self {
            agent: OnceLock::new(),
            bases,
        }
    }
}

impl GitHubApi for HttpGitHub {
    fn viewer(&self, d: &Domain, t: &SecretToken) -> Result<Org, ApiError> {
        let _ = (&self.agent, &self.bases, d, t);
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
