//! Folding fetch results into the previous cache (pure, infallible).

use crate::cache::Cache;
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
    },
}

/// True when `outcomes` is empty, or every domain is `Failed` or `Fetched`
/// with the personal job and every org failed. Refresh writes nothing then.
#[must_use]
pub fn all_failed(outcomes: &[DomainOutcome]) -> bool {
    let _ = outcomes;
    unimplemented!("W3.1")
}

/// Builds the new cache.
///
/// - A failed org carries forward its previous `(domain, org)` repos.
/// - A failed personal job carries forward the domain's previous repos whose
///   owner is not an org fetched successfully in this run.
/// - A failed domain carries forward all its previous repos.
/// - Domains not in `configured` and orgs no longer selected are dropped.
/// - Repos are de-duplicated by slug; the org-phase copy wins.
/// - Frecency and query frecency are moved out of `prev` (no clones) and
///   garbage-collected against the merged repo list.
#[must_use]
pub fn merge(
    prev: Option<Cache>,
    outcomes: Vec<DomainOutcome>,
    configured: &[Domain],
    now: StoredTime,
) -> Cache {
    let _ = (prev, outcomes, configured, now);
    unimplemented!("W3.1")
}
