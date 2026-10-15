//! The per-repo pipeline (fetch, facts, classify, fast-forward) and the
//! bounded `sync_all`.

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use crate::core::sync_outcome::{Plan, SyncOutcome, classify};
use crate::domain::Domain;
use crate::git::{Git, GitError};
use crate::pool::run_bounded;
use crate::progress::{Noun, Step, Tasks, Verb};
use crate::sync::SyncError;

/// One repo for [`sync_all`]: where it is and the domain git authenticates
/// for when fetching it.
#[derive(Debug)]
pub struct SyncTarget<'t> {
    /// The repo's path relative to `work_dir`.
    pub path: PathBuf,
    /// The repo's cached domain (not its remote's host).
    pub domain: &'t Domain,
}

/// One line of the `sync` report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncReport {
    /// The repo's path relative to `work_dir`.
    pub path: PathBuf,
    /// What happened.
    pub outcome: SyncOutcome,
}

/// Fetches `repo` (authenticating for `domain`), reads its facts,
/// classifies, and fast-forwards when the plan says so, reporting each step
/// to `on_step`. Every failure becomes `SyncOutcome::Error` (one line); this
/// never returns early.
#[must_use]
pub fn pull_one(
    git: &dyn Git,
    repo: &Path,
    domain: &Domain,
    on_step: &dyn Fn(Step),
) -> SyncOutcome {
    let fail = |err: &GitError| SyncOutcome::Error(crate::text::sanitize(&err.to_string()));
    on_step(Step::Fetching);
    if let Err(err) = git.fetch(repo, domain) {
        return fail(&err);
    }
    let facts = match git.facts(repo) {
        Ok(facts) => facts,
        Err(err) => return fail(&err),
    };
    match classify(&facts) {
        Plan::Done(outcome) => outcome,
        Plan::FastForward { commits } => {
            on_step(Step::FastForwarding);
            match git.fast_forward(repo) {
                Ok(()) => SyncOutcome::Pulled { commits },
                Err(err) => fail(&err),
            }
        }
    }
}

/// The `SomeFailed` error for `reports`, or `None` when no repo errored.
/// `failed` holds the relative paths of the errored repos, sorted.
#[must_use]
pub fn failures(reports: &[SyncReport]) -> Option<SyncError> {
    let mut failed: Vec<PathBuf> = reports
        .iter()
        .filter(|r| matches!(r.outcome, SyncOutcome::Error(_)))
        .map(|r| r.path.clone())
        .collect();
    if failed.is_empty() {
        return None;
    }
    failed.sort();
    Some(SyncError::SomeFailed {
        failed,
        total: reports.len(),
    })
}

/// Runs [`pull_one`] over `repos` with `jobs` workers, reporting a `Header`
/// and one task per repo to `tasks`. Each repo is fetched with its own
/// [`SyncTarget::domain`]. Results are sorted by relative path.
#[must_use]
pub fn sync_all(
    git: &dyn Git,
    work_dir: &Path,
    repos: &[SyncTarget<'_>],
    jobs: NonZeroUsize,
    tasks: &Tasks,
) -> Vec<SyncReport> {
    tasks.set_total(
        Verb::Syncing,
        u32::try_from(repos.len()).unwrap_or(u32::MAX),
        Noun::Repositories,
        None,
    );
    let mut reports = run_bounded(repos, jobs, |target| {
        let task = tasks.start(&target.path.display().to_string());
        let path = work_dir.join(&target.path);
        let on_step = |step: Step| task.detail(step.label());
        let outcome = pull_one(git, &path, target.domain, &on_step);
        match &outcome {
            SyncOutcome::Error(reason) => task.fail(reason),
            _ => task.ok(None),
        }
        SyncReport {
            path: target.path.clone(),
            outcome,
        }
    });
    reports.sort_by(|a, b| a.path.cmp(&b.path));
    reports
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::cell::RefCell;

    use super::*;
    use crate::core::sync_outcome::{FetchedReason, RepoFacts, Upstream};
    use crate::domain::Domain;
    use crate::progress::{Header, TaskDetail, TaskOutcome, UiEvent};
    use crate::testing::git::{FakeGit, GitOp, RepoScript};

    fn dom(s: &str) -> Domain {
        s.parse().expect("domain")
    }

    fn behind(n: u32) -> RepoFacts {
        RepoFacts::Branch {
            dirty: false,
            upstream: Upstream::Tracking {
                ahead: 0,
                behind: n,
            },
        }
    }

    fn script(facts: RepoFacts) -> RepoScript {
        RepoScript { facts, fail: None }
    }

    fn failing(op: GitOp, detail: &str) -> (GitOp, GitError) {
        (
            op,
            GitError::Failed {
                action: "fetch",
                detail: detail.to_owned(),
            },
        )
    }

    fn one() -> NonZeroUsize {
        NonZeroUsize::new(1).expect("nonzero")
    }

    fn rels(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    /// Targets for `names`, all on `domain`.
    fn targets<'d>(names: &[&str], domain: &'d Domain) -> Vec<SyncTarget<'d>> {
        names
            .iter()
            .map(|n| SyncTarget {
                path: PathBuf::from(n),
                domain,
            })
            .collect()
    }

    #[test]
    fn pull_one_fast_forwards_only_when_behind() {
        let w = Path::new("/w");
        let git = FakeGit::new()
            .with_repo(&w.join("a"), script(behind(3)))
            .with_repo(&w.join("b"), script(behind(0)));
        let steps = RefCell::new(Vec::new());
        let on_step = |s| steps.borrow_mut().push(s);
        assert_eq!(
            pull_one(&git, &w.join("a"), &dom("github.com"), &on_step),
            SyncOutcome::Pulled { commits: 3 }
        );
        assert_eq!(*steps.borrow(), [Step::Fetching, Step::FastForwarding]);
        assert_eq!(
            pull_one(&git, &w.join("b"), &dom("github.com"), &|_| {}),
            SyncOutcome::UpToDate
        );
        let ff = git
            .calls()
            .iter()
            .filter(|c| c.op == GitOp::FastForward)
            .count();
        assert_eq!(ff, 1);
    }

    #[test]
    fn pull_one_dirty_is_fetched_without_fast_forward() {
        let w = Path::new("/w/a");
        let facts = RepoFacts::Branch {
            dirty: true,
            upstream: Upstream::Tracking {
                ahead: 0,
                behind: 2,
            },
        };
        let git = FakeGit::new().with_repo(w, script(facts));
        assert_eq!(
            pull_one(&git, w, &dom("github.com"), &|_| {}),
            SyncOutcome::Fetched(FetchedReason::DirtyWorktree)
        );
        assert!(git.calls().iter().all(|c| c.op != GitOp::FastForward));
    }

    #[test]
    fn pull_one_errors_are_outcomes_in_order() {
        for (op, expect_calls) in [
            (GitOp::Fetch, 1),
            (GitOp::Facts, 2),
            (GitOp::FastForward, 3),
        ] {
            let w = Path::new("/w/a");
            let mut s = script(behind(1));
            s.fail = Some(failing(op, "boom"));
            let git = FakeGit::new().with_repo(w, s);
            let outcome = pull_one(&git, w, &dom("github.com"), &|_| {});
            assert_matches!(outcome, SyncOutcome::Error(ref m) if m.contains("boom"));
            assert_eq!(git.calls().len(), expect_calls);
        }
    }

    #[test]
    fn each_repo_is_fetched_for_its_own_domain() {
        let (github, ghe) = (dom("github.com"), dom("ghe.example.com"));
        let w = Path::new("/w");
        let repos = [
            SyncTarget {
                path: PathBuf::from("acme/a"),
                domain: &github,
            },
            SyncTarget {
                path: PathBuf::from("ghe.example.com/acme/b"),
                domain: &ghe,
            },
        ];
        let git = FakeGit::new();
        let (sink, _rx) = Tasks::channel();
        let reports = sync_all(&git, w, &repos, one(), &sink);
        assert_eq!(reports.len(), 2);
        let fetches: Vec<_> = git
            .calls()
            .into_iter()
            .filter(|c| c.op == GitOp::Fetch)
            .map(|c| (c.path, c.host))
            .collect();
        assert_eq!(
            fetches,
            [
                (w.join("acme/a"), Some(github)),
                (w.join("ghe.example.com/acme/b"), Some(ghe)),
            ]
        );
    }

    #[test]
    fn errors_are_isolated_sorted_and_events_emitted() {
        let w = Path::new("/w");
        let mut bad = script(behind(1));
        bad.fail = Some(failing(GitOp::Fetch, "boom"));
        let git = FakeGit::new()
            .with_repo(&w.join("b/bad"), bad)
            .with_repo(&w.join("a/ok"), script(behind(2)));
        let (sink, rx) = Tasks::channel();
        let github = dom("github.com");
        let repos = targets(&["b/bad", "a/ok"], &github);
        let reports = sync_all(&git, w, &repos, one(), &sink);
        drop(sink);
        assert_eq!(reports[0].path, PathBuf::from("a/ok"));
        assert_eq!(reports[0].outcome, SyncOutcome::Pulled { commits: 2 });
        assert_matches!(&reports[1].outcome, SyncOutcome::Error(_));

        let events: Vec<UiEvent> = rx.try_iter().collect();
        assert_matches!(
            &events[0],
            UiEvent::Header(Header {
                verb: Verb::Syncing,
                total: 2,
                noun: Noun::Repositories,
                ..
            })
        );
        // jobs = 1 runs in input order: bad is id 0, ok is id 1.
        assert_matches!(&events[1], UiEvent::TaskStarted { id: 0, label } if label == "b/bad");
        assert_matches!(
            &events[2],
            UiEvent::TaskProgress {
                id: 0,
                detail: TaskDetail::Text(t)
            } if t == Step::Fetching.label()
        );
        assert_matches!(
            &events[3],
            UiEvent::TaskFinished {
                id: 0,
                outcome: TaskOutcome::Failed { .. }
            }
        );
        assert_matches!(&events[4], UiEvent::TaskStarted { id: 1, .. });
        assert!(events.iter().any(|e| matches!(
            e,
            UiEvent::TaskProgress {
                id: 1,
                detail: TaskDetail::Text(t)
            } if t == Step::FastForwarding.label()
        )));
        assert_matches!(
            events.last(),
            Some(UiEvent::TaskFinished {
                id: 1,
                outcome: TaskOutcome::Ok { repos: None }
            })
        );
    }

    #[test]
    fn pool_limit_is_respected_and_order_is_deterministic() {
        let names: Vec<String> = (0..24).map(|i| format!("o/r{i:02}")).collect();
        let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let github = dom("github.com");
        let repos = targets(&name_refs, &github);
        let (sink, _rx) = Tasks::channel();
        let git = FakeGit::new();
        let jobs = NonZeroUsize::new(3).expect("nonzero");
        let reports = sync_all(&git, Path::new("/w"), &repos, jobs, &sink);
        assert_eq!(reports.len(), 24);
        assert!(git.max_in_flight() <= 3);
        let paths: Vec<_> = reports.into_iter().map(|r| r.path).collect();
        assert_eq!(paths, rels(&name_refs));
    }

    #[test]
    fn failures_lists_errored_repos_sorted() {
        let r = |p: &str, outcome| SyncReport {
            path: PathBuf::from(p),
            outcome,
        };
        let reports = [
            r("b/y", SyncOutcome::Error("x".into())),
            r("a/ok", SyncOutcome::UpToDate),
            r("a/x", SyncOutcome::Error("y".into())),
        ];
        assert_matches!(
            failures(&reports),
            Some(SyncError::SomeFailed { failed, total: 3 })
                if failed == rels(&["a/x", "b/y"])
        );
        assert_matches!(failures(&reports[1..2]), None);
    }
}
