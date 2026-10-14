//! The per-repo pipeline (fetch, facts, classify, fast-forward) and the
//! bounded `sync_all`.

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use crate::auth::{GitAuth, Tokens};
use crate::core::sync_outcome::{Plan, SyncOutcome, classify};
use crate::domain::Domain;
use crate::git::remote::{host_of, is_https};
use crate::git::{Git, GitError};
use crate::pool::run_bounded;
use crate::progress::{Header, Noun, ProgressSink, Step, TaskDetail, TaskOutcome, UiEvent, Verb};
use crate::sync::SyncError;

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
    SyncOutcome::Error(text.replace(char::is_control, " "))
}

/// The auth for `path`: only when `origin` is https, its host is one of
/// `configured`, and a token resolved for it.
fn auth_for<'t>(
    git: &dyn Git,
    path: &Path,
    tokens: &'t Tokens<'_>,
    configured: &[Domain],
) -> Option<GitAuth<'t>> {
    // An unreadable origin means the domain is unknown: fetch without auth
    // and let fetch/facts report a genuinely broken repo.
    let origin = git.origin_url(path).ok().flatten();
    let host = origin
        .as_deref()
        .filter(|url| is_https(url))
        .and_then(host_of)
        .filter(|host| configured.contains(host))?;
    // No usable token (an invalid `GITHUB_TOKEN` included) means no auth:
    // the fetch still works for public repos and stored credentials.
    tokens.git_auth(&host).ok().flatten()
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
/// and per-repo task events to `sink`. A repo gets a token only when its
/// `origin` is https and its host is in `configured`. Results are sorted by
/// relative path.
#[must_use]
pub fn sync_all(
    git: &dyn Git,
    work_dir: &Path,
    repos: &[PathBuf],
    tokens: &Tokens<'_>,
    configured: &[Domain],
    jobs: NonZeroUsize,
    sink: &ProgressSink,
) -> Vec<SyncReport> {
    sink.send(UiEvent::Header(Header {
        verb: Verb::Syncing,
        total: u32::try_from(repos.len()).unwrap_or(u32::MAX),
        noun: Noun::Repositories,
        domains: None,
    }));
    let indexed: Vec<(u32, &PathBuf)> = (0..).zip(repos).collect();
    let mut reports = run_bounded(&indexed, jobs, |&(id, rel)| {
        sink.send(UiEvent::TaskStarted {
            id,
            label: rel.display().to_string(),
        });
        let path = work_dir.join(rel);
        let on_step = |step| {
            sink.send(UiEvent::TaskProgress {
                id,
                detail: TaskDetail::Step(step),
            });
        };
        let auth = auth_for(git, &path, tokens, configured);
        let outcome = pull_one(git, &path, auth.as_ref(), &on_step);
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
            path: rel.clone(),
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
    use crate::auth::{EnvToken, SecretToken};
    use crate::core::sync_outcome::{FetchedReason, RepoFacts, Upstream};
    use crate::testing::gh::FakeGh;
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

    fn script(facts: RepoFacts, origin: Option<&str>) -> RepoScript {
        RepoScript {
            facts,
            origin: origin.map(str::to_owned),
            fail: None,
        }
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

    #[test]
    fn pull_one_fast_forwards_only_when_behind() {
        let w = Path::new("/w");
        let git = FakeGit::new()
            .with_repo(&w.join("a"), script(behind(3), None))
            .with_repo(&w.join("b"), script(behind(0), None));
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
        let git = FakeGit::new().with_repo(w, script(facts, None));
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
            let mut s = script(behind(1), None);
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
        let mut s = script(behind(1), None);
        s.fail = Some(failing(GitOp::Fetch, "bad gho_supersecret\nline2"));
        let git = FakeGit::new().with_repo(w, s);
        let outcome = pull_one(&git, w, Some(&auth), &|_| {});
        assert_matches!(outcome, SyncOutcome::Error(ref m)
            if !m.contains("gho_supersecret") && !m.contains('\n'));
    }

    #[test]
    fn token_only_for_configured_https_hosts() {
        let env = EnvToken::Valid(SecretToken::new("gho_tok").expect("token"));
        let tokens = Tokens::resolve(&FakeGh::new(), &env, &[]);
        let w = Path::new("/w");
        let git = FakeGit::new()
            .with_repo(
                &w.join("https"),
                script(behind(0), Some("https://github.com/a/https.git")),
            )
            .with_repo(
                &w.join("ssh"),
                script(behind(0), Some("git@github.com:a/ssh.git")),
            )
            .with_repo(
                &w.join("other"),
                script(behind(0), Some("https://example.com/a/other.git")),
            )
            .with_repo(&w.join("none"), script(behind(0), None));
        let (sink, _rx) = ProgressSink::channel();
        let repos = rels(&["https", "none", "other", "ssh"]);
        let reports = sync_all(&git, w, &repos, &tokens, &[dom("github.com")], one(), &sink);
        assert_eq!(reports.len(), 4);
        let fetches: Vec<_> = git
            .calls()
            .into_iter()
            .filter(|c| c.op == GitOp::Fetch)
            .collect();
        assert_eq!(fetches.len(), 4);
        for call in fetches {
            let expected = (call.path == w.join("https")).then(|| dom("github.com"));
            assert_eq!(call.auth_host, expected, "{:?}", call.path);
        }
    }

    #[test]
    fn errors_are_isolated_sorted_and_events_emitted() {
        let env = EnvToken::Unset;
        let tokens = Tokens::resolve(&FakeGh::new(), &env, &[]);
        let w = Path::new("/w");
        let mut bad = script(behind(1), None);
        bad.fail = Some(failing(GitOp::Fetch, "boom"));
        let git = FakeGit::new()
            .with_repo(&w.join("b/bad"), bad)
            .with_repo(&w.join("a/ok"), script(behind(2), None));
        let (sink, rx) = ProgressSink::channel();
        let repos = rels(&["b/bad", "a/ok"]);
        let reports = sync_all(&git, w, &repos, &tokens, &[], one(), &sink);
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
        let env = EnvToken::Unset;
        let tokens = Tokens::resolve(&FakeGh::new(), &env, &[]);
        let names: Vec<String> = (0..24).map(|i| format!("o/r{i:02}")).collect();
        let repos: Vec<PathBuf> = names.iter().map(PathBuf::from).collect();
        let (sink, _rx) = ProgressSink::channel();
        let git = FakeGit::new();
        let jobs = NonZeroUsize::new(3).expect("nonzero");
        let reports = sync_all(&git, Path::new("/w"), &repos, &tokens, &[], jobs, &sink);
        assert_eq!(reports.len(), 24);
        assert!(git.max_in_flight() <= 3);
        let paths: Vec<_> = reports.into_iter().map(|r| r.path).collect();
        assert_eq!(paths, repos);
    }

    #[test]
    fn origin_url_failure_still_fetches_without_auth() {
        let env = EnvToken::Valid(SecretToken::new("gho_tok").expect("token"));
        let tokens = Tokens::resolve(&FakeGh::new(), &env, &[]);
        let w = Path::new("/w");
        let mut s = script(behind(0), Some("https://github.com/a/x.git"));
        s.fail = Some((
            GitOp::OriginUrl,
            GitError::Failed {
                action: "remote",
                detail: "broken".to_owned(),
            },
        ));
        let git = FakeGit::new().with_repo(&w.join("x"), s);
        let (sink, _rx) = ProgressSink::channel();
        let repos = rels(&["x"]);
        let reports = sync_all(&git, w, &repos, &tokens, &[dom("github.com")], one(), &sink);
        assert_eq!(reports[0].outcome, SyncOutcome::UpToDate);
        let fetch = git.calls().into_iter().find(|c| c.op == GitOp::Fetch);
        assert_matches!(fetch, Some(c) if c.auth_host.is_none());
    }

    #[test]
    fn upstream_only_repo_fetches_without_auth() {
        // `origin_url` reports `Ok(None)` when there is no `origin` remote.
        let env = EnvToken::Valid(SecretToken::new("gho_tok").expect("token"));
        let tokens = Tokens::resolve(&FakeGh::new(), &env, &[]);
        let w = Path::new("/w");
        let git = FakeGit::new().with_repo(&w.join("u"), script(behind(1), None));
        let (sink, _rx) = ProgressSink::channel();
        let repos = rels(&["u"]);
        let reports = sync_all(&git, w, &repos, &tokens, &[dom("github.com")], one(), &sink);
        assert_eq!(reports[0].outcome, SyncOutcome::Pulled { commits: 1 });
        assert!(git.calls().iter().all(|c| c.auth_host.is_none()));
    }

    #[test]
    fn http_is_unauthenticated_and_uppercase_https_host_matches() {
        let w = Path::new("/w");
        let git = FakeGit::new()
            .with_repo(
                &w.join("plain"),
                script(behind(0), Some("http://github.com/a/p.git")),
            )
            .with_repo(
                &w.join("upper"),
                script(behind(0), Some("HTTPS://GitHub.com/a/u.git")),
            );
        let (sink, _rx) = ProgressSink::channel();
        let repos = rels(&["plain", "upper"]);
        let env = EnvToken::Valid(SecretToken::new("gho_tok").expect("token"));
        let tokens = Tokens::resolve(&FakeGh::new(), &env, &[]);
        let _ = sync_all(&git, w, &repos, &tokens, &[dom("github.com")], one(), &sink);
        let hosts: Vec<_> = git
            .calls()
            .into_iter()
            .filter(|c| c.op == GitOp::Fetch)
            .map(|c| (c.path, c.auth_host))
            .collect();
        assert_eq!(
            hosts,
            [
                (w.join("plain"), None),
                (w.join("upper"), Some(dom("github.com")))
            ]
        );
    }

    #[test]
    fn failed_token_lookup_still_fetches_without_auth() {
        let w = Path::new("/w");
        let git = FakeGit::new().with_repo(
            &w.join("r"),
            script(behind(0), Some("https://github.com/a/r.git")),
        );
        let (sink, _rx) = ProgressSink::channel();
        let tokens = Tokens::resolve(&FakeGh::new(), &EnvToken::Invalid, &[]);
        let repos = rels(&["r"]);
        let reports = sync_all(&git, w, &repos, &tokens, &[dom("github.com")], one(), &sink);
        assert_eq!(reports[0].outcome, SyncOutcome::UpToDate);
        assert!(git.calls().iter().all(|c| c.auth_host.is_none()));
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
