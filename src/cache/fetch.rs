//! The concurrent fetch: domains in parallel, then every listing job in one
//! bounded pool across all domains, each a `gh api` call.

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
    /// Owners filtered out (see `DomainOutcome::Fetched::deselected`).
    deselected: Vec<Org>,
}

/// What one pool job lists.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum JobKind {
    /// `user_repos`: owner and collaborator repos.
    Personal,
    /// `org_repos` for this org.
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

/// Fetches everything in `plan` through `gh`.
///
/// First the domain phase, with the domains in parallel (up to
/// [`FETCH_JOBS`] at once, so one unreachable host does not hold up the
/// others): `viewer`, `member_orgs`, `select_orgs`. A failure there makes
/// that domain `DomainOutcome::Failed` and warns. Then, when there is at
/// least one job, sets the header total (every job, the personal ones
/// included, as `Noun::Sources` defines, across the domains in `plan`), and
/// runs one personal job per ready domain plus every selected org through
/// `pool::run_bounded` with [`FETCH_JOBS`] workers. Each job is a task whose
/// detail is its running repo count. Failed jobs and skipped lines warn.
///
/// The outcomes are in plan order.
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

/// The domain phase for one domain, on a pool worker. It sends nothing; the
/// caller warns about failures.
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

/// Runs one job on a pool worker and returns its result under its key. It
/// reports only through `tasks`.
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

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::sync::mpsc;

    use super::*;
    use crate::progress::{TaskDetail, TaskOutcome, UiEvent};
    use crate::testing::{Endpoint, FakeGh};

    fn dom(s: &str) -> Domain {
        s.parse().unwrap()
    }

    fn org(s: &str) -> Org {
        s.parse().unwrap()
    }

    fn orgs(names: &[&str]) -> Vec<Org> {
        names.iter().map(|n| org(n)).collect()
    }

    fn repo(d: &str, owner: &str, name: &str) -> Repo {
        serde_json::from_value(serde_json::json!({
            "slug": format!("{d}/{owner}/{name}"),
            "clone_url": format!("https://{d}/{owner}/{name}.git"),
            "html_url": format!("https://{d}/{owner}/{name}"),
        }))
        .unwrap()
    }

    fn plan_for<'c>(domains: Vec<DomainPlan<'c>>, exclude: &'c [Org]) -> FetchPlan<'c> {
        FetchPlan {
            domains,
            include: &[],
            exclude,
            all_orgs: false,
        }
    }

    fn bare(d: &str) -> DomainPlan<'static> {
        DomainPlan {
            domain: dom(d),
            extra: &[],
            favorites: &[],
        }
    }

    /// Runs `fetch_all` and returns the outcomes plus every event sent.
    fn run(plan: &FetchPlan<'_>, gh: &FakeGh) -> (Vec<DomainOutcome>, Vec<UiEvent>) {
        let (tasks, rx) = Tasks::channel();
        let out = fetch_all(plan, gh, &tasks);
        drop(tasks);
        (out, drain(&rx))
    }

    fn drain(rx: &mpsc::Receiver<UiEvent>) -> Vec<UiEvent> {
        rx.try_iter().collect()
    }

    fn warnings(events: &[UiEvent]) -> Vec<&str> {
        events
            .iter()
            .filter_map(|e| match e {
                UiEvent::Warn(w) => Some(w.as_str()),
                _ => None,
            })
            .collect()
    }

    fn slugs(outcome: &OrgOutcome) -> Vec<String> {
        match outcome {
            OrgOutcome::Fetched(repos) => repos.iter().map(|r| r.slug.to_string()).collect(),
            OrgOutcome::Failed(cause) => panic!("expected Fetched, got Failed({cause})"),
        }
    }

    fn fetched(outcome: &DomainOutcome) -> (&OrgOutcome, &[(Org, OrgOutcome)]) {
        match outcome {
            DomainOutcome::Fetched { personal, orgs, .. } => (personal, orgs),
            DomainOutcome::Failed { cause, .. } => panic!("expected Fetched, got Failed({cause})"),
        }
    }

    fn org_names(list: &[(Org, OrgOutcome)]) -> Vec<&str> {
        list.iter().map(|(o, _)| o.as_str()).collect()
    }

    #[test]
    fn results_land_under_their_own_domain_and_org() {
        let mut gh = FakeGh::new();
        for d in ["github.com", "ghe.example.com"] {
            gh = gh
                .with_viewer(dom(d), org("me"))
                .with_member_orgs(dom(d), orgs(&["acme", "beta"]))
                .with_user_repos(dom(d), vec![repo(d, "me", "mine")])
                .with_org_repos(dom(d), org("acme"), vec![repo(d, "acme", "a")])
                .with_org_repos(dom(d), org("beta"), vec![repo(d, "beta", "b")]);
        }
        let plan = plan_for(vec![bare("github.com"), bare("ghe.example.com")], &[]);
        let (out, events) = run(&plan, &gh);
        assert_eq!(out.len(), 2);
        for (outcome, d) in out.iter().zip(["github.com", "ghe.example.com"]) {
            assert_matches!(outcome, DomainOutcome::Fetched { domain, .. } if domain.as_str() == d);
            let (personal, orgs) = fetched(outcome);
            assert_eq!(slugs(personal), [format!("{d}/me/mine")]);
            assert_eq!(org_names(orgs), ["acme", "beta"]);
            assert_eq!(slugs(&orgs[0].1), [format!("{d}/acme/a")]);
            assert_eq!(slugs(&orgs[1].1), [format!("{d}/beta/b")]);
        }
        assert_eq!(warnings(&events), Vec::<&str>::new());
    }

    #[test]
    fn collaborator_repos_are_deduplicated_and_filtered_by_owner() {
        let d = "github.com";
        let gh = FakeGh::new()
            .with_viewer(dom(d), org("me"))
            .with_member_orgs(dom(d), orgs(&["acme"]))
            .with_user_repos(
                dom(d),
                vec![
                    repo(d, "me", "own"),
                    repo(d, "acme", "shared"),
                    repo(d, "blocked", "collab"),
                    repo(d, "friend", "collab"),
                ],
            )
            .with_org_repos(dom(d), org("acme"), vec![repo(d, "acme", "shared")]);
        // Excluding the viewer never drops their own repos.
        let exclude = orgs(&["BLOCKED", "me"]);
        let plan = plan_for(vec![bare(d)], &exclude);
        let (out, _) = run(&plan, &gh);
        let (personal, orgs) = fetched(&out[0]);
        assert_eq!(
            slugs(personal),
            ["github.com/me/own", "github.com/friend/collab"]
        );
        assert_eq!(slugs(&orgs[0].1), ["github.com/acme/shared"]);
    }

    #[test]
    fn task_detail_is_the_running_repo_count() {
        let d = "github.com";
        let gh = FakeGh::new()
            .with_viewer(dom(d), org("me"))
            .with_user_repos(
                dom(d),
                vec![repo(d, "me", "a"), repo(d, "me", "b"), repo(d, "me", "c")],
            );
        let plan = plan_for(vec![bare(d)], &[]);
        let (_, events) = run(&plan, &gh);
        let details: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                UiEvent::TaskProgress {
                    detail: TaskDetail::Text(t),
                    ..
                } => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(details, ["1 repo", "2 repos", "3 repos"]);
    }

    #[test]
    fn org_failure_gives_failed_outcome_and_warning_with_ghs_message() {
        let d = "github.com";
        let gh = FakeGh::new()
            .with_viewer(dom(d), org("me"))
            .with_member_orgs(dom(d), orgs(&["acme", "beta"]))
            .with_user_repos(dom(d), vec![repo(d, "acme", "shared")])
            .with_org_repos(dom(d), org("beta"), vec![repo(d, "beta", "q")])
            .with_failure(
                dom(d),
                Endpoint::OrgRepos(org("acme")),
                "gh: Not Found (HTTP 404)",
            );
        let plan = plan_for(vec![bare(d)], &[]);
        let (out, events) = run(&plan, &gh);
        let (personal, orgs) = fetched(&out[0]);
        assert_matches!(&orgs[0], (o, OrgOutcome::Failed(cause)) if o.as_str() == "acme" && cause == "gh: Not Found (HTTP 404)");
        assert_eq!(slugs(&orgs[1].1), ["github.com/beta/q"]);
        // A collaborator copy is kept when its org job failed.
        assert_eq!(slugs(personal), ["github.com/acme/shared"]);
        let warns = warnings(&events);
        assert_eq!(warns.len(), 1);
        assert!(
            warns[0].contains("github.com/acme") && warns[0].contains("HTTP 404"),
            "{warns:?}"
        );
        assert!(events.iter().any(|e| matches!(
            e,
            UiEvent::TaskFinished { outcome: TaskOutcome::Failed { reason }, .. }
                if reason.contains("HTTP 404")
        )));
    }

    #[test]
    fn viewer_failure_fails_only_that_domain_with_a_warning() {
        let gh = FakeGh::new()
            .with_viewer(dom("github.com"), org("me"))
            .with_failure(
                dom("ghe.example.com"),
                Endpoint::Viewer,
                "gh: Bad credentials (HTTP 401)",
            );
        let plan = plan_for(vec![bare("ghe.example.com"), bare("github.com")], &[]);
        let (out, events) = run(&plan, &gh);
        assert_matches!(
            &out[0],
            DomainOutcome::Failed { domain, cause }
                if domain.as_str() == "ghe.example.com" && cause.contains("HTTP 401")
        );
        assert_matches!(&out[1], DomainOutcome::Fetched { .. });
        let warns = warnings(&events);
        assert_eq!(warns.len(), 1);
        assert!(
            warns[0].contains("ghe.example.com") && warns[0].contains("HTTP 401"),
            "{warns:?}"
        );
    }

    #[test]
    fn skipped_lines_warn_once_per_listing() {
        let d = "github.com";
        let gh = FakeGh::new()
            .with_viewer(dom(d), org("me"))
            .with_member_orgs(dom(d), orgs(&["acme"]))
            .with_org_repos(dom(d), org("acme"), vec![repo(d, "acme", "x")])
            .with_skipped(dom(d), Endpoint::OrgRepos(org("acme")), 2);
        let plan = plan_for(vec![bare(d)], &[]);
        let (out, events) = run(&plan, &gh);
        assert_eq!(slugs(&fetched(&out[0]).1[0].1), ["github.com/acme/x"]);
        let warns = warnings(&events);
        assert_eq!(warns.len(), 1, "{warns:?}");
        assert!(
            warns[0].contains("github.com/acme") && warns[0].contains("skipped 2 repos"),
            "{warns:?}"
        );
    }

    fn deselected_of(outcome: &DomainOutcome) -> Vec<&str> {
        match outcome {
            DomainOutcome::Fetched { deselected, .. } => {
                deselected.iter().map(Org::as_str).collect()
            }
            DomainOutcome::Failed { cause, .. } => panic!("expected Fetched, got Failed({cause})"),
        }
    }

    #[test]
    fn deselected_lists_unselected_member_and_extra_orgs_once() {
        let d = "github.com";
        let gh = FakeGh::new()
            .with_viewer(dom(d), org("me"))
            .with_member_orgs(dom(d), orgs(&["acme", "beta", "blocked"]));
        let extra = orgs(&["partner", "Partner"]);
        let favorites = orgs(&["beta"]);
        let exclude = orgs(&["BLOCKED", "outsider"]);
        let plan = plan_for(
            vec![DomainPlan {
                domain: dom(d),
                extra: &extra,
                favorites: &favorites,
            }],
            &exclude,
        );
        let (out, _) = run(&plan, &gh);
        assert_eq!(org_names(fetched(&out[0]).1), ["beta"]);
        // Each org once, case-insensitively; the collaborator-only
        // `outsider` is left to `merge`, which applies the filters itself.
        assert_eq!(deselected_of(&out[0]), ["acme", "blocked", "partner"]);
    }
}
