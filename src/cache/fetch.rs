//! The concurrent fetch: domains in order, then every org job in one bounded
//! pool across all domains.

use std::collections::HashSet;
use std::num::NonZeroUsize;

use crate::auth::{SecretToken, Tokens};
use crate::cache::merge::{DomainOutcome, OrgOutcome};
use crate::config::Config;
use crate::core::org_filter::{OrgSelection, passes_filters, select_orgs};
use crate::domain::{Domain, Org, Slug};
use crate::github::{GitHubApi, MAX_PAGES, PER_PAGE, Page};
use crate::pool::run_bounded;
use crate::progress::{Header, Noun, ProgressSink, TaskDetail, TaskOutcome, UiEvent, Verb};

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
enum Prepared<'p> {
    /// The domain phase failed; the cause is one line.
    Failed { domain: &'p Domain, cause: String },
    /// Ready for the org phase.
    Ready {
        domain: &'p Domain,
        ready: Ready<'p>,
    },
}

/// What a successful domain phase found.
struct Ready<'t> {
    token: &'t SecretToken,
    viewer: Org,
    orgs: Vec<Org>,
    /// Owners filtered out (see `DomainOutcome::Fetched::deselected`).
    deselected: Vec<Org>,
}

/// What one pool job lists.
#[derive(Debug, Clone, Copy)]
enum JobKind<'p> {
    /// `user_repos`: owner and collaborator repos.
    Personal,
    /// `org_repos` for this org.
    Org(&'p Org),
}

/// One unit of work for the org phase.
#[derive(Debug)]
struct Job<'p> {
    id: u32,
    domain: &'p Domain,
    token: &'p SecretToken,
    kind: JobKind<'p>,
}

impl Job<'_> {
    /// The progress label, e.g. `github.com/acme` or
    /// `github.com/@me (personal)`.
    fn label(&self) -> String {
        match self.kind {
            JobKind::Personal => format!("{}/@me (personal)", self.domain),
            JobKind::Org(org) => format!("{}/{org}", self.domain),
        }
    }
}

/// Fetches everything in `plan`.
///
/// First the domain phase, with the domains in parallel (up to
/// [`FETCH_JOBS`] at once, so one unreachable host does not hold up the
/// others): token (none → `DomainOutcome::Failed` plus a warning),
/// `viewer`, `member_orgs`, `select_orgs`. Then, when there is at least one
/// job, sends one `Header` (its `total` counts every job, the personal ones
/// included, as `Noun::Sources` defines; its `domains` is the number of domains
/// in `plan`), and runs one personal job per ready domain plus every
/// selected org through `pool::run_bounded` with [`FETCH_JOBS`] workers,
/// each reporting `TaskStarted`, `TaskProgress` per page, and
/// `TaskFinished`. Failures, truncated listings and skipped repos also send
/// a `Warn`.
///
/// # Panics
///
/// Only if `pool::run_bounded` broke its one-result-per-item contract.
#[must_use]
pub fn fetch_all(
    plan: &FetchPlan<'_>,
    api: &dyn GitHubApi,
    tokens: &Tokens<'_>,
    sink: &ProgressSink,
) -> Vec<DomainOutcome> {
    let ready = run_bounded(&plan.domains, FETCH_JOBS, |dp| {
        prepare_domain(plan, dp, api, tokens)
    });
    let prepared: Vec<Prepared<'_>> = plan
        .domains
        .iter()
        .zip(ready)
        .map(|(dp, ready)| match ready {
            Ok(ready) => Prepared::Ready {
                domain: &dp.domain,
                ready,
            },
            Err(cause) => Prepared::Failed {
                domain: &dp.domain,
                cause,
            },
        })
        .collect();
    // Warn from this thread, in domain order, so the output is stable.
    for p in &prepared {
        if let Prepared::Failed { domain, cause } = p {
            sink.warn(format!("cannot refresh {domain}: {cause}"));
        }
    }

    let jobs: Vec<Job<'_>> = (0..)
        .zip(
            prepared
                .iter()
                .filter_map(|p| match p {
                    Prepared::Ready { domain, ready } => Some((*domain, ready.token, &ready.orgs)),
                    Prepared::Failed { .. } => None,
                })
                .flat_map(|(domain, token, orgs)| {
                    std::iter::once(JobKind::Personal)
                        .chain(orgs.iter().map(JobKind::Org))
                        .map(move |kind| (domain, token, kind))
                }),
        )
        .map(|(id, (domain, token, kind))| Job {
            id,
            domain,
            token,
            kind,
        })
        .collect();

    if !jobs.is_empty() {
        sink.send(UiEvent::Header(Header {
            verb: Verb::Updating,
            total: to_u32(jobs.len()),
            noun: Noun::Sources,
            domains: Some(to_u32(plan.domains.len())),
        }));
    }

    let results = run_bounded(&jobs, FETCH_JOBS, |job| run_job(job, api, sink));
    drop(jobs);

    // Jobs were queued per ready domain as the personal job, then each org,
    // and `run_bounded` returns one result per job, in input order.
    let mut results = results.into_iter();
    let mut next_result = || {
        results
            .next()
            .expect("run_bounded returns one result per job")
    };
    prepared
        .into_iter()
        .map(|p| match p {
            Prepared::Failed { domain, cause } => DomainOutcome::Failed {
                domain: domain.clone(),
                cause,
            },
            Prepared::Ready {
                domain,
                ready:
                    Ready {
                        viewer,
                        orgs,
                        deselected,
                        ..
                    },
            } => {
                let personal = next_result();
                let orgs: Vec<(Org, OrgOutcome)> =
                    orgs.into_iter().map(|org| (org, next_result())).collect();
                let personal = filter_personal(personal, &viewer, &orgs, plan);
                DomainOutcome::Fetched {
                    domain: domain.clone(),
                    viewer,
                    personal,
                    orgs,
                    deselected,
                }
            }
        })
        .collect()
}

/// The domain phase for one domain, on a pool worker. It sends nothing; the
/// caller warns about failures.
fn prepare_domain<'t>(
    plan: &FetchPlan<'_>,
    dp: &DomainPlan<'_>,
    api: &dyn GitHubApi,
    tokens: &'t Tokens<'_>,
) -> Result<Ready<'t>, String> {
    let domain = &dp.domain;
    tokens
        .get(domain)
        .map_err(|e| e.to_string())
        .and_then(|(token, _)| {
            let viewer = api.viewer(domain, token).map_err(|e| e.to_string())?;
            let member = api.member_orgs(domain, token).map_err(|e| e.to_string())?;
            let orgs = select_orgs(&OrgSelection {
                member: &member,
                extra: dp.extra,
                include: plan.include,
                exclude: plan.exclude,
                favorites: dp.favorites,
                all_orgs: plan.all_orgs,
            });
            // Collaborator-only owners the global filters reject are
            // handled by `merge`, which applies the filters itself.
            let mut deselected: Vec<Org> = Vec::new();
            for org in member.into_iter().chain(dp.extra.iter().cloned()) {
                let known = |list: &[Org]| list.iter().any(|o| o.eq_ignore_case(&org));
                if !known(&orgs) && !known(&deselected) {
                    deselected.push(org);
                }
            }
            Ok(Ready {
                token,
                viewer,
                orgs,
                deselected,
            })
        })
}

/// Runs one job on a pool worker. It reports only through `sink`.
fn run_job(job: &Job<'_>, api: &dyn GitHubApi, sink: &ProgressSink) -> OrgOutcome {
    let id = job.id;
    let label = job.label();
    sink.send(UiEvent::TaskStarted {
        id,
        label: label.clone(),
    });
    let mut on_page = |page: Page| {
        sink.send(UiEvent::TaskProgress {
            id,
            detail: TaskDetail::Page {
                page: page.number,
                repos: page.repos_so_far,
            },
        });
    };
    let listing = match job.kind {
        JobKind::Personal => api.user_repos(job.domain, job.token, &mut on_page),
        JobKind::Org(org) => api.org_repos(job.domain, job.token, org, &mut on_page),
    };
    match listing {
        Ok(listing) => {
            if listing.truncated {
                sink.warn(format!(
                    "{label} truncated at {} repos",
                    MAX_PAGES.saturating_mul(PER_PAGE)
                ));
            }
            if listing.skipped > 0 {
                sink.warn(format!(
                    "{label}: skipped {} repos with invalid data",
                    listing.skipped
                ));
            }
            sink.send(UiEvent::TaskFinished {
                id,
                outcome: TaskOutcome::Ok {
                    repos: Some(to_u32(listing.repos.len())),
                },
            });
            OrgOutcome::Fetched(listing.repos)
        }
        Err(err) => {
            let reason = err.to_string();
            sink.warn(format!("cannot fetch {label}: {reason}"));
            sink.send(UiEvent::TaskFinished {
                id,
                outcome: TaskOutcome::Failed {
                    reason: reason.clone(),
                },
            });
            OrgOutcome::Failed(reason)
        }
    }
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
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Condvar, Mutex, mpsc};
    use std::time::Duration;

    use super::*;
    use crate::auth::{EnvToken, SecretToken};
    use crate::config::{ConfigFile, DomainConfig, Filters, Locations};
    use crate::domain::Repo;
    use crate::env::EnvSnapshot;
    use crate::github::{ApiError, Listing};
    use crate::testing::{Endpoint, FakeGh, FakeGitHub};

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

    fn env_token() -> EnvToken {
        EnvToken::Valid(SecretToken::new("ghp_test").unwrap())
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
    fn run(
        plan: &FetchPlan<'_>,
        api: &dyn GitHubApi,
        env: &EnvToken,
        gh: &FakeGh,
    ) -> (Vec<DomainOutcome>, Vec<UiEvent>) {
        let domains: Vec<Domain> = plan.domains.iter().map(|d| d.domain.clone()).collect();
        let tokens = Tokens::resolve(gh, env, &domains);
        let (sink, rx) = ProgressSink::channel();
        let out = fetch_all(plan, api, &tokens, &sink);
        drop(sink);
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
    fn from_config_uses_configured_domains_and_filters() {
        let env = EnvSnapshot {
            home: Some("/home/u".into()),
            ..EnvSnapshot::default()
        };
        let locations = Locations::from_env(&env).unwrap();
        let mut domains = BTreeMap::new();
        domains.insert(
            dom("ghe.example.com"),
            DomainConfig {
                extra_orgs: orgs(&["partner"]),
                favorite_orgs: orgs(&["fav"]),
                ..DomainConfig::default()
            },
        );
        let file = ConfigFile {
            filters: Filters {
                include_orgs: orgs(&["inc"]),
                exclude_orgs: orgs(&["exc"]),
            },
            domains,
            ..ConfigFile::default()
        };
        let config = Config::from_file(file, &locations, Path::new("/c.toml")).unwrap();
        let plan = FetchPlan::from_config(&config, true);
        assert!(plan.all_orgs);
        assert_eq!(plan.include, orgs(&["inc"]).as_slice());
        assert_eq!(plan.exclude, orgs(&["exc"]).as_slice());
        assert_eq!(plan.domains.len(), 1);
        assert_eq!(plan.domains[0].domain, dom("ghe.example.com"));
        assert_eq!(plan.domains[0].extra, orgs(&["partner"]).as_slice());
        assert_eq!(plan.domains[0].favorites, orgs(&["fav"]).as_slice());

        let empty =
            Config::from_file(ConfigFile::default(), &locations, Path::new("/c.toml")).unwrap();
        let plan = FetchPlan::from_config(&empty, false);
        assert_eq!(plan.domains.len(), 1);
        assert_eq!(plan.domains[0].domain, dom("github.com"));
        assert!(plan.domains[0].extra.is_empty() && plan.domains[0].favorites.is_empty());
    }

    /// Wraps a fake so the first `need` listings wait for each other inside
    /// their first page (while the fake counts them as in flight), or, with
    /// `on_viewer`, the first `need` `viewer` calls wait before calling it.
    #[derive(Debug)]
    struct Rendezvous<'a> {
        inner: &'a FakeGitHub,
        need: usize,
        on_viewer: bool,
        arrived: Mutex<usize>,
        cv: Condvar,
        timed_out: AtomicBool,
    }

    impl Rendezvous<'_> {
        fn meet(&self) {
            let mut n = self.arrived.lock().unwrap();
            if *n >= self.need {
                return;
            }
            *n += 1;
            self.cv.notify_all();
            // The timeout is only a hang guard: the test passes as soon as
            // `need` callers arrive, whatever the clock does, and a timeout
            // fails the test instead of blocking it forever.
            let (_guard, res) = self
                .cv
                .wait_timeout_while(n, Duration::from_secs(10), |c| *c < self.need)
                .unwrap();
            if res.timed_out() {
                self.timed_out.store(true, Ordering::SeqCst);
            }
        }
    }

    impl GitHubApi for Rendezvous<'_> {
        fn viewer(&self, d: &Domain, t: &SecretToken) -> Result<Org, ApiError> {
            if self.on_viewer {
                self.meet();
            }
            self.inner.viewer(d, t)
        }

        fn member_orgs(&self, d: &Domain, t: &SecretToken) -> Result<Vec<Org>, ApiError> {
            self.inner.member_orgs(d, t)
        }

        fn user_repos(
            &self,
            d: &Domain,
            t: &SecretToken,
            on_page: &mut dyn FnMut(Page),
        ) -> Result<Listing, ApiError> {
            self.inner.user_repos(d, t, &mut |p| {
                if !self.on_viewer {
                    self.meet();
                }
                on_page(p);
            })
        }

        fn org_repos(
            &self,
            d: &Domain,
            t: &SecretToken,
            o: &Org,
            on_page: &mut dyn FnMut(Page),
        ) -> Result<Listing, ApiError> {
            self.inner.org_repos(d, t, o, &mut |p| {
                if !self.on_viewer {
                    self.meet();
                }
                on_page(p);
            })
        }
    }

    #[test]
    fn at_most_eight_requests_in_flight_across_domains_and_eight_reached() {
        let names = ["o1", "o2", "o3", "o4", "o5", "o6"];
        let mut fake = FakeGitHub::new();
        for d in ["github.com", "ghe.example.com"] {
            fake = fake
                .with_viewer(dom(d), org("me"))
                .with_member_orgs(dom(d), orgs(&names))
                .with_user_repos(dom(d), vec![vec![repo(d, "me", "mine")]]);
            for n in names {
                fake = fake.with_org_repos(dom(d), org(n), vec![vec![repo(d, n, "r")]]);
            }
        }
        let api = Rendezvous {
            inner: &fake,
            need: FETCH_JOBS.get(),
            on_viewer: false,
            arrived: Mutex::new(0),
            cv: Condvar::new(),
            timed_out: AtomicBool::new(false),
        };
        let plan = plan_for(vec![bare("github.com"), bare("ghe.example.com")], &[]);
        let (out, _) = run(&plan, &api, &env_token(), &FakeGh::new());
        assert!(
            !api.timed_out.load(Ordering::SeqCst),
            "fewer than 8 overlapped"
        );
        assert_eq!(fake.max_in_flight(), 8);
        assert_eq!(out.len(), 2);
        for outcome in &out {
            let (_, orgs) = fetched(outcome);
            assert_eq!(org_names(orgs), names);
        }
    }

    #[test]
    fn one_personal_job_per_domain() {
        let mut fake = FakeGitHub::new();
        for d in ["github.com", "ghe.example.com"] {
            fake = fake
                .with_viewer(dom(d), org("me"))
                .with_member_orgs(dom(d), orgs(&["acme"]))
                .with_user_repos(dom(d), vec![vec![repo(d, "me", "mine")]]);
        }
        let plan = plan_for(vec![bare("github.com"), bare("ghe.example.com")], &[]);
        let (out, _) = run(&plan, &fake, &env_token(), &FakeGh::new());
        let personal_calls: Vec<Domain> = fake
            .calls()
            .into_iter()
            .filter(|c| c.endpoint == Endpoint::UserRepos)
            .map(|c| c.domain)
            .collect();
        assert_eq!(personal_calls.len(), 2);
        assert!(personal_calls.contains(&dom("github.com")));
        assert!(personal_calls.contains(&dom("ghe.example.com")));
        for (outcome, d) in out.iter().zip(["github.com", "ghe.example.com"]) {
            let (personal, _) = fetched(outcome);
            assert_eq!(slugs(personal), [format!("{d}/me/mine")]);
        }
    }

    #[test]
    fn collaborator_repos_are_deduplicated_and_filtered_by_owner() {
        let d = "github.com";
        let fake = FakeGitHub::new()
            .with_viewer(dom(d), org("me"))
            .with_member_orgs(dom(d), orgs(&["acme"]))
            .with_user_repos(
                dom(d),
                vec![vec![
                    repo(d, "me", "own"),
                    repo(d, "acme", "shared"),
                    repo(d, "blocked", "collab"),
                    repo(d, "friend", "collab"),
                ]],
            )
            .with_org_repos(dom(d), org("acme"), vec![vec![repo(d, "acme", "shared")]]);
        // Excluding the viewer never drops their own repos.
        let exclude = orgs(&["BLOCKED", "me"]);
        let plan = plan_for(vec![bare(d)], &exclude);
        let (out, _) = run(&plan, &fake, &env_token(), &FakeGh::new());
        let (personal, orgs) = fetched(&out[0]);
        assert_eq!(
            slugs(personal),
            ["github.com/me/own", "github.com/friend/collab"]
        );
        assert_eq!(slugs(&orgs[0].1), ["github.com/acme/shared"]);
    }

    #[test]
    fn collaborator_repo_is_kept_when_its_org_job_failed() {
        let d = "github.com";
        let fake = FakeGitHub::new()
            .with_viewer(dom(d), org("me"))
            .with_member_orgs(dom(d), orgs(&["acme"]))
            .with_user_repos(dom(d), vec![vec![repo(d, "acme", "shared")]])
            .with_status(dom(d), Endpoint::OrgRepos(org("acme")), 502);
        let plan = plan_for(vec![bare(d)], &[]);
        let (out, _) = run(&plan, &fake, &env_token(), &FakeGh::new());
        let (personal, _) = fetched(&out[0]);
        assert_eq!(slugs(personal), ["github.com/acme/shared"]);
    }

    /// (label, (page, repos) per page, final repo count).
    type ExpectedTask = (&'static str, &'static [(u32, u32)], u32);

    #[test]
    fn event_stream_is_header_then_started_pages_finished_per_job() {
        let d = "github.com";
        let fake = FakeGitHub::new()
            .with_viewer(dom(d), org("me"))
            .with_member_orgs(dom(d), orgs(&["acme", "beta"]))
            .with_user_repos(
                dom(d),
                vec![
                    vec![repo(d, "me", "a")],
                    vec![repo(d, "me", "b")],
                    vec![repo(d, "me", "c")],
                ],
            )
            .with_org_repos(
                dom(d),
                org("acme"),
                vec![
                    vec![repo(d, "acme", "x"), repo(d, "acme", "y")],
                    vec![repo(d, "acme", "z")],
                ],
            )
            .with_org_repos(dom(d), org("beta"), vec![vec![repo(d, "beta", "q")]]);
        let plan = plan_for(vec![bare(d)], &[]);
        let (_, events) = run(&plan, &fake, &env_token(), &FakeGh::new());

        assert_matches!(
            events.first(),
            Some(UiEvent::Header(Header {
                verb: Verb::Updating,
                total: 3,
                noun: Noun::Sources,
                domains: Some(1),
            }))
        );
        let headers = events
            .iter()
            .filter(|e| matches!(e, UiEvent::Header(_)))
            .count();
        assert_eq!(headers, 1);

        let mut per_task: BTreeMap<u32, Vec<&UiEvent>> = BTreeMap::new();
        for event in &events {
            let id = match event {
                UiEvent::TaskStarted { id, .. }
                | UiEvent::TaskProgress { id, .. }
                | UiEvent::TaskFinished { id, .. } => *id,
                UiEvent::Header(_) | UiEvent::Warn(_) => continue,
            };
            per_task.entry(id).or_default().push(event);
        }
        let expected: [ExpectedTask; 3] = [
            ("github.com/@me (personal)", &[(1, 1), (2, 2), (3, 3)], 3),
            ("github.com/acme", &[(1, 2), (2, 3)], 3),
            ("github.com/beta", &[(1, 1)], 1),
        ];
        assert_eq!(per_task.len(), 3);
        for ((&id, task), (label, pages, total)) in per_task.iter().zip(expected) {
            assert_eq!(task.len(), pages.len() + 2, "{task:?}");
            assert_matches!(task[0], UiEvent::TaskStarted { label: l, .. } if l == label);
            for (event, &(page, repos)) in task[1..].iter().zip(pages) {
                assert_eq!(
                    **event,
                    UiEvent::TaskProgress {
                        id,
                        detail: TaskDetail::Page { page, repos },
                    }
                );
            }
            assert_matches!(
                task.last(),
                Some(UiEvent::TaskFinished {
                    outcome: TaskOutcome::Ok { repos: Some(n) },
                    ..
                }) if *n == total
            );
        }
        assert_eq!(warnings(&events), Vec::<&str>::new());
    }

    #[test]
    fn org_failure_gives_failed_outcome_and_warning_naming_org_and_cause() {
        let d = "github.com";
        let fake = FakeGitHub::new()
            .with_viewer(dom(d), org("me"))
            .with_member_orgs(dom(d), orgs(&["acme", "beta"]))
            .with_org_repos(dom(d), org("beta"), vec![vec![repo(d, "beta", "q")]])
            .with_status(dom(d), Endpoint::OrgRepos(org("acme")), 502);
        let plan = plan_for(vec![bare(d)], &[]);
        let (out, events) = run(&plan, &fake, &env_token(), &FakeGh::new());
        let (_, orgs) = fetched(&out[0]);
        assert_matches!(&orgs[0], (o, OrgOutcome::Failed(cause)) if o.as_str() == "acme" && cause.contains("HTTP 502"));
        assert_eq!(slugs(&orgs[1].1), ["github.com/beta/q"]);
        let warns = warnings(&events);
        assert_eq!(warns.len(), 1);
        assert!(
            warns[0].contains("github.com/acme") && warns[0].contains("HTTP 502"),
            "{warns:?}"
        );
        assert!(events.iter().any(|e| matches!(
            e,
            UiEvent::TaskFinished { outcome: TaskOutcome::Failed { reason }, .. }
                if reason.contains("HTTP 502")
        )));
    }

    #[test]
    fn missing_token_fails_only_that_domain_with_a_warning() {
        let fake = FakeGitHub::new()
            .with_viewer(dom("github.com"), org("me"))
            .with_member_orgs(dom("github.com"), Vec::new());
        let gh = FakeGh::new().with_token(dom("github.com"), "gho_ok");
        let plan = plan_for(vec![bare("ghe.example.com"), bare("github.com")], &[]);
        let (out, events) = run(&plan, &fake, &EnvToken::Unset, &gh);
        assert_matches!(
            &out[0],
            DomainOutcome::Failed { domain, cause }
                if domain.as_str() == "ghe.example.com" && cause.contains("no GitHub token")
        );
        assert_matches!(&out[1], DomainOutcome::Fetched { .. });
        assert!(
            fake.calls()
                .iter()
                .all(|c| c.domain.as_str() == "github.com")
        );
        let warns = warnings(&events);
        assert_eq!(warns.len(), 1);
        assert!(
            warns[0].contains("ghe.example.com") && warns[0].contains("no GitHub token"),
            "{warns:?}"
        );
        assert_matches!(
            events.iter().find(|e| matches!(e, UiEvent::Header(_))),
            Some(UiEvent::Header(Header {
                total: 1,
                domains: Some(2),
                ..
            }))
        );
    }

    #[test]
    fn viewer_failure_fails_the_domain_with_a_warning() {
        let fake = FakeGitHub::new()
            .with_viewer(dom("github.com"), org("me"))
            .with_status(dom("github.com"), Endpoint::Viewer, 401);
        let plan = plan_for(vec![bare("github.com")], &[]);
        let (out, events) = run(&plan, &fake, &env_token(), &FakeGh::new());
        assert_matches!(&out[0], DomainOutcome::Failed { cause, .. } if cause.contains("HTTP 401"));
        assert_eq!(fake.calls().len(), 1);
        let warns = warnings(&events);
        assert!(warns[0].contains("github.com") && warns[0].contains("HTTP 401"));
        // Nothing to run, so no header.
        assert_matches!(
            events.iter().find(|e| matches!(e, UiEvent::Header(_))),
            None
        );
    }

    #[test]
    fn extra_orgs_are_deduplicated() {
        let d = "github.com";
        let fake = FakeGitHub::new()
            .with_viewer(dom(d), org("me"))
            .with_member_orgs(dom(d), orgs(&["Acme"]));
        let extra = orgs(&["acme", "partner", "PARTNER"]);
        let plan = plan_for(
            vec![DomainPlan {
                domain: dom(d),
                extra: &extra,
                favorites: &[],
            }],
            &[],
        );
        let (out, _) = run(&plan, &fake, &env_token(), &FakeGh::new());
        let (_, orgs) = fetched(&out[0]);
        assert_eq!(org_names(orgs), ["Acme", "partner"]);
        let org_calls = fake
            .calls()
            .into_iter()
            .filter(|c| matches!(c.endpoint, Endpoint::OrgRepos(_)))
            .count();
        assert_eq!(org_calls, 2);
    }

    #[test]
    fn all_orgs_ignores_favorites() {
        let d = "github.com";
        let fake = FakeGitHub::new()
            .with_viewer(dom(d), org("me"))
            .with_member_orgs(dom(d), orgs(&["acme", "beta"]));
        let favorites = orgs(&["beta"]);
        let mut plan = plan_for(
            vec![DomainPlan {
                domain: dom(d),
                extra: &[],
                favorites: &favorites,
            }],
            &[],
        );
        let (out, _) = run(&plan, &fake, &env_token(), &FakeGh::new());
        assert_eq!(org_names(fetched(&out[0]).1), ["beta"]);

        plan.all_orgs = true;
        let (out, events) = run(&plan, &fake, &env_token(), &FakeGh::new());
        assert_eq!(org_names(fetched(&out[0]).1), ["acme", "beta"]);
        assert_matches!(
            events.first(),
            Some(UiEvent::Header(Header { total: 3, .. }))
        );
    }

    #[test]
    fn domain_phases_run_concurrently() {
        let mut fake = FakeGitHub::new();
        for d in ["github.com", "ghe.example.com"] {
            fake = fake.with_viewer(dom(d), org("me"));
        }
        let api = Rendezvous {
            inner: &fake,
            need: 2,
            on_viewer: true,
            arrived: Mutex::new(0),
            cv: Condvar::new(),
            timed_out: AtomicBool::new(false),
        };
        let plan = plan_for(vec![bare("github.com"), bare("ghe.example.com")], &[]);
        let (out, _) = run(&plan, &api, &env_token(), &FakeGh::new());
        assert!(
            !api.timed_out.load(Ordering::SeqCst),
            "domain phases ran one at a time"
        );
        // Outcomes stay in plan order.
        assert_matches!(&out[0], DomainOutcome::Fetched { domain, .. } if domain.as_str() == "github.com");
        assert_matches!(&out[1], DomainOutcome::Fetched { domain, .. } if domain.as_str() == "ghe.example.com");
    }

    #[test]
    fn header_counts_every_job_and_every_planned_domain() {
        let fake = FakeGitHub::new()
            .with_viewer(dom("github.com"), org("me"))
            .with_member_orgs(dom("github.com"), orgs(&["acme", "beta", "gamma"]))
            .with_status(dom("ghe.example.com"), Endpoint::Viewer, 500);
        let plan = plan_for(vec![bare("github.com"), bare("ghe.example.com")], &[]);
        let (_, events) = run(&plan, &fake, &env_token(), &FakeGh::new());
        let headers: Vec<&Header> = events
            .iter()
            .filter_map(|e| match e {
                UiEvent::Header(h) => Some(h),
                _ => None,
            })
            .collect();
        // 3 orgs plus the personal job; both configured domains.
        assert_matches!(
            headers.as_slice(),
            [Header {
                total: 4,
                domains: Some(2),
                ..
            }]
        );
    }

    /// Wraps a fake and marks one org's listing as truncated with skipped
    /// entries, as the HTTP client reports them.
    #[derive(Debug)]
    struct Doctored<'a> {
        inner: &'a FakeGitHub,
        org: Org,
    }

    impl GitHubApi for Doctored<'_> {
        fn viewer(&self, d: &Domain, t: &SecretToken) -> Result<Org, ApiError> {
            self.inner.viewer(d, t)
        }

        fn member_orgs(&self, d: &Domain, t: &SecretToken) -> Result<Vec<Org>, ApiError> {
            self.inner.member_orgs(d, t)
        }

        fn user_repos(
            &self,
            d: &Domain,
            t: &SecretToken,
            on_page: &mut dyn FnMut(Page),
        ) -> Result<Listing, ApiError> {
            self.inner.user_repos(d, t, on_page)
        }

        fn org_repos(
            &self,
            d: &Domain,
            t: &SecretToken,
            o: &Org,
            on_page: &mut dyn FnMut(Page),
        ) -> Result<Listing, ApiError> {
            let mut listing = self.inner.org_repos(d, t, o, on_page)?;
            if *o == self.org {
                listing.truncated = true;
                listing.skipped = 2;
            }
            Ok(listing)
        }
    }

    #[test]
    fn truncated_and_skipped_listings_warn_before_the_job_finishes() {
        let d = "github.com";
        let fake = FakeGitHub::new()
            .with_viewer(dom(d), org("me"))
            .with_member_orgs(dom(d), orgs(&["acme", "beta"]))
            .with_org_repos(dom(d), org("acme"), vec![vec![repo(d, "acme", "x")]]);
        let api = Doctored {
            inner: &fake,
            org: org("acme"),
        };
        let plan = plan_for(vec![bare(d)], &[]);
        let (out, events) = run(&plan, &api, &env_token(), &FakeGh::new());
        assert_eq!(slugs(&fetched(&out[0]).1[0].1), ["github.com/acme/x"]);

        let acme_id = events
            .iter()
            .find_map(|e| match e {
                UiEvent::TaskStarted { id, label } if label == "github.com/acme" => Some(*id),
                _ => None,
            })
            .unwrap();
        let finished_at = events
            .iter()
            .position(|e| matches!(e, UiEvent::TaskFinished { id, .. } if *id == acme_id))
            .unwrap();
        let warned: Vec<(usize, &str)> = events
            .iter()
            .enumerate()
            .filter_map(|(i, e)| match e {
                UiEvent::Warn(w) => Some((i, w.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(warned.len(), 2, "{warned:?}");
        assert!(
            warned
                .iter()
                .all(|(i, w)| *i < finished_at && w.contains("github.com/acme")),
            "{warned:?}"
        );
        assert!(
            warned[0].1.contains("truncated at 10000 repos"),
            "{warned:?}"
        );
        assert!(warned[1].1.contains("skipped 2 repos"), "{warned:?}");
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
        let fake = FakeGitHub::new()
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
        let (out, _) = run(&plan, &fake, &env_token(), &FakeGh::new());
        assert_eq!(org_names(fetched(&out[0]).1), ["beta"]);
        // Each org once, case-insensitively; the collaborator-only
        // `outsider` is left to `merge`, which applies the filters itself.
        assert_eq!(deselected_of(&out[0]), ["acme", "blocked", "partner"]);
    }

    #[test]
    fn deselected_lists_member_orgs_outside_include_orgs() {
        let d = "github.com";
        let fake = FakeGitHub::new()
            .with_viewer(dom(d), org("me"))
            .with_member_orgs(dom(d), orgs(&["acme", "beta"]));
        let include = orgs(&["beta"]);
        let exclude = orgs(&["beta", "outsider"]);
        let plan = FetchPlan {
            domains: vec![bare(d)],
            include: &include,
            exclude: &exclude,
            all_orgs: false,
        };
        let (out, _) = run(&plan, &fake, &env_token(), &FakeGh::new());
        assert_eq!(org_names(fetched(&out[0]).1), ["beta"]);
        // Collaborator-only owners outside `include_orgs` are not listed
        // here: the personal job filters them, and so does `merge` when it
        // fails.
        assert_eq!(deselected_of(&out[0]), ["acme"]);
    }
}
