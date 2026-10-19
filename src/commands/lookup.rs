//! Looking an `org/repo` or `domain/org/repo` query up on GitHub when no
//! known repo matches it exactly.

use crate::cache::fetch::FETCH_JOBS;
use crate::config::Config;
use crate::core::search::{Query, Specificity};
use crate::domain::{Domain, Org, Repo, RepoName, Slug};
use crate::gh::{Gh, Lookup};
use crate::pool::run_bounded;

/// One query to look up: the same owner and name on each of `domains`.
#[derive(Debug)]
pub(crate) struct Probe {
    domains: Vec<Domain>,
    org: Org,
    name: RepoName,
}

/// One domain's answer.
#[derive(Debug)]
pub(crate) enum Answer {
    Found(Repo),
    NotFound,
    Denied(String),
    Failed(String),
}

impl Probe {
    /// The lookup `query` calls for: `None` for a `repo` query, a segment
    /// that is not a valid name, or a domain that is not configured.
    pub(crate) fn plan(config: &Config, query: &Query) -> Option<Self> {
        let mut segments = query.key().as_str().split('/');
        let domains = match query.specificity() {
            Specificity::Name => return None,
            Specificity::OrgName => config.effective_domains(),
            Specificity::Full => {
                let domain: Domain = segments.next()?.parse().ok()?;
                let configured = config.effective_domains().contains(&domain);
                configured.then(|| vec![domain])?
            }
        };
        let org = Org::try_from(segments.next()?.to_owned()).ok()?;
        let name = RepoName::try_from(segments.next()?.to_owned()).ok()?;
        Some(Self { domains, org, name })
    }

    /// The slug the query names on `domain`, for messages.
    pub(crate) fn slug_on(&self, domain: &Domain) -> Slug {
        Slug::new(domain.clone(), self.org.clone(), self.name.clone())
    }

    /// The domains looked up, in configuration order.
    pub(crate) fn domains(&self) -> &[Domain] {
        &self.domains
    }

    /// Asks every domain at once; answers come back in domain order.
    pub(crate) fn run(&self, gh: &dyn Gh) -> Vec<(Domain, Answer)> {
        let answers = run_bounded(&self.domains, FETCH_JOBS, |domain| {
            ask(gh, domain, &self.org, &self.name)
        });
        self.domains.iter().cloned().zip(answers).collect()
    }
}

/// Looks `org/name` up on `domain`.
pub(crate) fn ask(gh: &dyn Gh, domain: &Domain, org: &Org, name: &RepoName) -> Answer {
    match gh.repo(domain, org, name) {
        Ok(Lookup::Found(repo)) => Answer::Found(repo),
        Ok(Lookup::NotFound) => Answer::NotFound,
        Ok(Lookup::Denied(detail)) => Answer::Denied(detail),
        Err(err) => Answer::Failed(err.to_string()),
    }
}
