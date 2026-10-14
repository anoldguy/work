//! The per-repo pipeline (fetch, facts, classify, fast-forward) and the
//! bounded `sync_all`.

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use crate::auth::GitAuth;
use crate::core::sync_outcome::{Plan, SyncOutcome, classify};
use crate::git::{Git, GitError};
use crate::pool::run_bounded;
use crate::progress::{Header, Noun, ProgressSink, Step, TaskDetail, TaskOutcome, UiEvent, Verb};
use crate::sync::SyncError;

/// One repo for [`sync_all`]: where it is and the auth to fetch it with.
#[derive(Debug)]
pub struct SyncTarget<'t> {
    /// The repo's path relative to `work_dir`.
    pub path: PathBuf,
    /// The header for the repo's cached domain, scoped to that host; `None`
    /// fetches unauthenticated.
    pub auth: Option<GitAuth<'t>>,
}

/// One line of the `sync` report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncReport {
    /// The repo's path relative to `work_dir`.
    pub path: PathBuf,
    /// What happened.
    pub outcome: SyncOutcome,
}

/// Fetches `repo`, reads its facts, classifies, and fast-forwards when the
/// plan says so, reporting each step to `on_step`. Every failure becomes
/// `SyncOutcome::Error` (one line, redacted); this never returns early.
#[must_use]
pub fn pull_one(
    git: &dyn Git,
    repo: &Path,
    auth: Option<&GitAuth<'_>>,
    on_step: &dyn Fn(Step),
) -> SyncOutcome {
    let fail = |err: &GitError| error_outcome(err, auth);
    on_step(Step::Fetching);
    if let Err(err) = git.fetch(repo, auth) {
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

/// A one-line, redacted error outcome.
fn error_outcome(err: &GitError, auth: Option<&GitAuth<'_>>) -> SyncOutcome {
    let text = err.to_string();
    let text = match auth {
        Some(a) => a.redact(&text),
        None => text,
    };
    SyncOutcome::Error(crate::text::sanitize(&text))
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
/// and per-repo task events to `sink`. Each repo is fetched with its own
/// [`SyncTarget::auth`]. Results are sorted by relative path.
#[must_use]
pub fn sync_all(
    git: &dyn Git,
    work_dir: &Path,
    repos: &[SyncTarget<'_>],
    jobs: NonZeroUsize,
    sink: &ProgressSink,
) -> Vec<SyncReport> {
    sink.send(UiEvent::Header(Header {
        verb: Verb::Syncing,
        total: u32::try_from(repos.len()).unwrap_or(u32::MAX),
        noun: Noun::Repositories,
        domains: None,
    }));
    let indexed: Vec<(u32, &SyncTarget<'_>)> = (0..).zip(repos).collect();
    let mut reports = run_bounded(&indexed, jobs, |&(id, target)| {
        sink.send(UiEvent::TaskStarted {
            id,
            label: target.path.display().to_string(),
        });
        let path = work_dir.join(&target.path);
        let on_step = |step| {
            sink.send(UiEvent::TaskProgress {
                id,
                detail: TaskDetail::Step(step),
            });
        };
        let outcome = pull_one(git, &path, target.auth.as_ref(), &on_step);
        let task_outcome = match &outcome {
            SyncOutcome::Error(reason) => TaskOutcome::Failed {
                reason: reason.clone(),
            },
            _ => TaskOutcome::Ok { repos: None },
        };
        sink.send(UiEvent::TaskFinished {
            id,
            outcome: task_outcome,
        });
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
    use crate::auth::SecretToken;
    use crate::core::sync_outcome::{FetchedReason, RepoFacts, Upstream};
    use crate::domain::Domain;
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

    /// Unauthenticated targets for `names`.
    fn targets(names: &[&str]) -> Vec<SyncTarget<'static>> {
        names
            .iter()
            .map(|n| SyncTarget {
                path: PathBuf::from(n),
                auth: None,
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
            pull_one(&git, &w.join("a"), None, &on_step),
            SyncOutcome::Pulled { commits: 3 }
        );
        assert_eq!(*steps.borrow(), [Step::Fetching, Step::FastForwarding]);
        assert_eq!(
            pull_one(&git, &w.join("b"), None, &|_| {}),
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
            pull_one(&git, w, None, &|_| {}),
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
            let outcome = pull_one(&git, w, None, &|_| {});
            assert_matches!(outcome, SyncOutcome::Error(ref m) if m.contains("boom"));
            assert_eq!(git.calls().len(), expect_calls);
        }
    }

    #[test]
    fn pull_one_redacts_the_token_in_errors() {
        let token = SecretToken::new("gho_supersecret").expect("token");
        let auth = GitAuth::new(dom("github.com"), &token);
        let w = Path::new("/w/a");
        let mut s = script(behind(1));
        s.fail = Some(failing(GitOp::Fetch, "bad gho_supersecret\nline2"));
        let git = FakeGit::new().with_repo(w, s);
        let outcome = pull_one(&git, w, Some(&auth), &|_| {});
        assert_matches!(outcome, SyncOutcome::Error(ref m)
            if !m.contains("gho_supersecret") && !m.contains('\n'));
    }

    #[test]
    fn each_repo_is_fetched_with_its_own_auth() {
        let token = SecretToken::new("gho_tok").expect("token");
        let w = Path::new("/w");
        let repos = [
            SyncTarget {
                path: PathBuf::from("acme/a"),
                auth: Some(GitAuth::new(dom("github.com"), &token)),
            },
            SyncTarget {
                path: PathBuf::from("ghe.example.com/acme/b"),
                auth: Some(GitAuth::new(dom("ghe.example.com"), &token)),
            },
            SyncTarget {
                path: PathBuf::from("other.example.com/acme/c"),
                auth: None,
            },
        ];
        let git = FakeGit::new();
        let (sink, _rx) = ProgressSink::channel();
        let reports = sync_all(&git, w, &repos, one(), &sink);
        assert_eq!(reports.len(), 3);
        let fetches: Vec<_> = git
            .calls()
            .into_iter()
            .filter(|c| c.op == GitOp::Fetch)
            .map(|c| (c.path, c.auth_host))
            .collect();
        assert_eq!(
            fetches,
            [
                (w.join("acme/a"), Some(dom("github.com"))),
                (
                    w.join("ghe.example.com/acme/b"),
                    Some(dom("ghe.example.com"))
                ),
                (w.join("other.example.com/acme/c"), None),
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
        let (sink, rx) = ProgressSink::channel();
        let repos = targets(&["b/bad", "a/ok"]);
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
                detail: TaskDetail::Step(Step::Fetching)
            }
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
                detail: TaskDetail::Step(Step::FastForwarding)
            }
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
        let repos = targets(&name_refs);
        let (sink, _rx) = ProgressSink::channel();
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
