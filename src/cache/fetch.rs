//! The concurrent fetch: domains in order, then every org job in one bounded
//! pool across all domains.

use std::num::NonZeroUsize;

use crate::auth::Tokens;
use crate::cache::merge::DomainOutcome;
use crate::config::Config;
use crate::domain::{Domain, Org};
use crate::github::GitHubApi;
use crate::progress::ProgressSink;

/// Org jobs in flight at once, across all domains.
pub const FETCH_JOBS: NonZeroUsize = NonZeroUsize::new(8).expect("8 is non-zero");

/// What to fetch, borrowed from the config.
#[derive(Debug)]
pub struct FetchPlan<'c> {
    /// Effective domains, in order.
    pub domains: Vec<DomainPlan<'c>>,
    /// Global `include_orgs`.
    pub include: &'c [Org],
    /// Global `exclude_orgs`.
    pub exclude: &'c [Org],
    /// `update --all-orgs`.
    pub all_orgs: bool,
}

/// One domain's part of a [`FetchPlan`].
#[derive(Debug)]
pub struct DomainPlan<'c> {
    /// The domain (owned, because the implicit domain is not in the config).
    pub domain: Domain,
    /// Its `extra_orgs` (empty for the implicit domain).
    pub extra: &'c [Org],
    /// Its `favorite_orgs` (empty for the implicit domain).
    pub favorites: &'c [Org],
}

impl<'c> FetchPlan<'c> {
    /// The plan for every effective domain of `config`.
    #[must_use]
    pub fn from_config(config: &'c Config, all_orgs: bool) -> Self {
        let _ = (config, all_orgs);
        unimplemented!("W3.2")
    }
}

/// Fetches everything in `plan`.
///
/// Per domain, on the calling thread: token (none → `DomainOutcome::Failed`
/// plus a warning), `viewer`, `member_orgs`, `select_orgs`. Then sends one
/// `Header`, and runs one personal job per domain plus every selected org
/// through `pool::run_bounded` with [`FETCH_JOBS`] workers, each reporting
/// `TaskStarted`, `TaskProgress` per page, and `TaskFinished`. Failures also
/// send a `Warn`.
#[must_use]
pub fn fetch_all(
    plan: &FetchPlan<'_>,
    api: &dyn GitHubApi,
    tokens: &Tokens<'_>,
    sink: &ProgressSink,
) -> Vec<DomainOutcome> {
    let _ = (plan, api, tokens, sink);
    unimplemented!("W3.2")
}
