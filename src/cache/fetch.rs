//! The concurrent repository fetch through `gh`.

use std::collections::{BTreeMap, HashSet};
use std::num::NonZeroUsize;

use crate::cache::merge::{DomainOutcome, OrgOutcome};
use crate::config::Config;
use crate::core::org_filter::{OrgSelection, passes_filters, select_orgs};
use crate::domain::{Domain, Org, Repo, Slug};
use crate::gh::Gh;
use crate::pool::run_bounded;
use crate::progress::{Noun, Tasks, Verb, group};
use crate::text::plural;

/// Listing jobs in flight at once, across all domains.
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
    /// Ignore favourite orgs.
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
        let domains = config
            .effective_domains()
            .into_iter()
            .map(|domain| {
                let cfg = config.domains().get(&domain);
                DomainPlan {
                    extra: cfg.map_or(&[][..], |c| c.extra_orgs.as_slice()),
                    favorites: cfg.map_or(&[][..], |c| c.favorite_orgs.as_slice()),
                    domain,
                }
            })
            .collect();
        let filters = config.filters();
        Self {
            domains,
            include: &filters.include_orgs,
            exclude: &filters.exclude_orgs,
            all_orgs,
        }
    }
}

/// One domain after the domain phase.
enum Prepared {
    /// The domain phase failed; the cause is one line.
    Failed { domain: Domain, cause: String },
    /// Ready for the org phase.
    Ready { domain: Domain, ready: Ready },
}

/// What a successful domain phase found.
struct Ready {
    viewer: Org,
    orgs: Vec<Org>,
    /// Owners filtered out.
    deselected: Vec<Org>,
}

/// What one pool job lists.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum JobKind {
    /// The viewer's owner and collaborator repos.
    Personal,
    /// One org's repos.
    Org(Org),
}

/// Which listing a job ran. Every job result carries its key, so results
/// are grouped by key, never by position.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct JobKey {
    /// The domain listed.
    pub domain: Domain,
    /// Personal or one org.
    pub kind: JobKind,
}

impl JobKey {
    /// The progress label, e.g. `github.com/acme` or
    /// `github.com/@me (personal)`.
    fn label(&self) -> String {
        match &self.kind {
            JobKind::Personal => format!("{}/@me (personal)", self.domain),
            JobKind::Org(org) => format!("{}/{org}", self.domain),
        }
    }
}

/// One domain's job results, gathered by key.
#[derive(Default)]
struct Gathered {
    personal: Option<OrgOutcome>,
    orgs: Vec<(Org, OrgOutcome)>,
}

/// Fetches everything in `plan` through `gh`, one outcome per domain in plan
/// order.
///
/// Domains are prepared in parallel so one unreachable host does not hold up
/// the others; then every listing job, across all domains, shares one pool of
/// [`FETCH_JOBS`] workers.
#[must_use]
pub fn fetch_all(plan: &FetchPlan<'_>, gh: &dyn Gh, tasks: &Tasks) -> Vec<DomainOutcome> {
    // Each result carries its own domain (cloned to cross the worker
    // boundary), so nothing depends on the pool's result order.
    let prepared: Vec<Prepared> = run_bounded(&plan.domains, FETCH_JOBS, |dp| {
        let domain = dp.domain.clone();
        match prepare_domain(plan, dp, gh) {
            Ok(ready) => Prepared::Ready { domain, ready },
            Err(cause) => Prepared::Failed { domain, cause },
        }
    });
    // Warn from this thread, in domain order, so the output is stable.
    for p in &prepared {
        if let Prepared::Failed { domain, cause } = p {
            tasks.warn(format!("cannot refresh {domain}: {cause}"));
        }
    }

    let jobs: Vec<JobKey> = prepared
        .iter()
        .filter_map(|p| match p {
            Prepared::Ready { domain, ready } => Some((domain, &ready.orgs)),
            Prepared::Failed { .. } => None,
        })
        .flat_map(|(domain, orgs)| {
            std::iter::once(JobKind::Personal)
                .chain(orgs.iter().cloned().map(JobKind::Org))
                .map(move |kind| JobKey {
                    domain: domain.clone(),
                    kind,
                })
        })
        .collect();

    if !jobs.is_empty() {
        tasks.set_total(
            Verb::Updating,
            to_u32(jobs.len()),
            Noun::Sources,
            Some(to_u32(plan.domains.len())),
        );
    }

    let mut gathered: BTreeMap<Domain, Gathered> = BTreeMap::new();
    for (key, outcome) in run_bounded(&jobs, FETCH_JOBS, |job| run_job(job, gh, tasks)) {
        let entry = gathered.entry(key.domain).or_default();
        match key.kind {
            JobKind::Personal => entry.personal = Some(outcome),
            JobKind::Org(org) => entry.orgs.push((org, outcome)),
        }
    }

    prepared
        .into_iter()
        .map(|p| match p {
            Prepared::Failed { domain, cause } => DomainOutcome::Failed { domain, cause },
            Prepared::Ready {
                domain,
                ready:
                    Ready {
                        viewer,
                        orgs: selected,
                        deselected,
                    },
            } => {
                let Gathered { personal, orgs } = gathered.remove(&domain).unwrap_or_default();
                let orgs = in_selection_order(orgs, &selected);
                // Every ready domain queued a personal job. Should its result
                // ever be missing, it counts as a failed listing, which
                // carries the previous repos forward.
                let personal =
                    personal.unwrap_or_else(|| OrgOutcome::Failed("not fetched".to_owned()));
                let personal = filter_personal(personal, &viewer, &orgs, plan);
                DomainOutcome::Fetched {
                    domain,
                    viewer,
                    personal,
                    orgs,
                    deselected,
                }
            }
        })
        .collect()
}

/// `orgs` in the order `select_orgs` chose, so the output is stable.
fn in_selection_order(
    mut orgs: Vec<(Org, OrgOutcome)>,
    selected: &[Org],
) -> Vec<(Org, OrgOutcome)> {
    orgs.sort_by_key(|(org, _)| selected.iter().position(|s| s == org));
    orgs
}

/// The domain phase for one domain, on a pool worker. It reports nothing, so
/// warnings can be emitted in domain order.
fn prepare_domain(plan: &FetchPlan<'_>, dp: &DomainPlan<'_>, gh: &dyn Gh) -> Result<Ready, String> {
    let domain = &dp.domain;
    let viewer = gh.viewer(domain).map_err(|e| e.to_string())?;
    let member = gh.member_orgs(domain).map_err(|e| e.to_string())?;
    let orgs = select_orgs(&OrgSelection {
        member: &member,
        extra: dp.extra,
        include: plan.include,
        exclude: plan.exclude,
        favorites: dp.favorites,
        all_orgs: plan.all_orgs,
    });
    let mut deselected: Vec<Org> = Vec::new();
    for org in member.into_iter().chain(dp.extra.iter().cloned()) {
        let known = |list: &[Org]| list.iter().any(|o| o.eq_ignore_case(&org));
        if !known(&orgs) && !known(&deselected) {
            deselected.push(org);
        }
    }
    Ok(Ready {
        viewer,
        orgs,
        deselected,
    })
}

/// Runs one job on a pool worker and returns its result under its key.
fn run_job(job: &JobKey, gh: &dyn Gh, tasks: &Tasks) -> (JobKey, OrgOutcome) {
    let label = job.label();
    let task = tasks.start(&label);
    let mut repos: Vec<Repo> = Vec::new();
    let mut on_repo = |repo: Repo| {
        repos.push(repo);
        let n = repos.len();
        task.detail(&format!(
            "{} {}",
            group(u64::try_from(n).unwrap_or(u64::MAX)),
            plural(n, "repo", "repos")
        ));
    };
    let listing = match &job.kind {
        JobKind::Personal => gh.user_repos(&job.domain, &mut on_repo),
        JobKind::Org(org) => gh.org_repos(&job.domain, org, &mut on_repo),
    };
    let outcome = match listing {
        Ok(skipped) => {
            if skipped > 0 {
                tasks.warn(format!(
                    "{label}: skipped {skipped} {} with invalid data",
                    plural(skipped, "repo", "repos")
                ));
            }
            task.ok(Some(to_u32(repos.len())));
            OrgOutcome::Fetched(repos)
        }
        Err(err) => {
            let reason = err.to_string();
            tasks.warn(format!("cannot fetch {label}: {reason}"));
            task.fail(&reason);
            OrgOutcome::Failed(reason)
        }
    };
    (job.clone(), outcome)
}

/// Keeps the viewer's own repos, drops collaborator repos whose owner fails
/// the global filters, and drops repos an org job of this domain fetched
/// (the org-phase copy wins).
fn filter_personal(
    personal: OrgOutcome,
    viewer: &Org,
    orgs: &[(Org, OrgOutcome)],
    plan: &FetchPlan<'_>,
) -> OrgOutcome {
    let OrgOutcome::Fetched(mut repos) = personal else {
        return personal;
    };
    let org_slugs: HashSet<&Slug> = orgs
        .iter()
        .filter_map(|(_, outcome)| match outcome {
            OrgOutcome::Fetched(repos) => Some(repos),
            OrgOutcome::Failed(_) => None,
        })
        .flatten()
        .map(|repo| &repo.slug)
        .collect();
    repos.retain(|repo| {
        let owner = repo.slug.org();
        let wanted =
            owner.eq_ignore_case(viewer) || passes_filters(owner, plan.include, plan.exclude);
        wanted && !org_slugs.contains(&repo.slug)
    });
    OrgOutcome::Fetched(repos)
}

/// `n` as a `u32`, saturating (job and repo counts never get near it).
fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}
