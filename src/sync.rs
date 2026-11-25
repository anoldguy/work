//! `pull` and `sync`: fetch cloned repos, classify their status, and
//! fast-forward when safe.

use std::fmt;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use crate::domain::Domain;
use crate::git::{self, GitError, RepoFacts, Upstream};
use crate::pool::run_bounded;
use crate::text::plural;
use crate::ui::progress::{Noun, Step, Tasks, Verb};

/// Why a repo was fetched but not fast-forwarded. None of these is an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchedReason {
    /// Uncommitted or untracked changes.
    DirtyWorktree,
    /// Local commits that are not on the upstream.
    Diverged,
    /// Not on a branch.
    DetachedHead,
    /// The branch has no upstream.
    NoUpstream,
}

impl fmt::Display for FetchedReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::DirtyWorktree => "dirty worktree",
            Self::Diverged => "diverged",
            Self::DetachedHead => "detached HEAD",
            Self::NoUpstream => "no upstream",
        })
    }
}

/// The result of syncing one repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOutcome {
    /// Fast-forwarded by this many commits.
    Pulled {
        /// Commits fast-forwarded.
        commits: u32,
    },
    /// Clean and already at the upstream.
    UpToDate,
    /// Fetched only.
    Fetched(FetchedReason),
    /// Something failed; one line.
    Error(String),
}

impl fmt::Display for SyncOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pulled { commits } => {
                let noun = plural(*commits, "commit", "commits");
                write!(f, "pulled ({commits} {noun})")
            }
            Self::UpToDate => f.write_str("up to date"),
            Self::Fetched(reason) => write!(f, "fetched ({reason})"),
            Self::Error(message) => write!(f, "error: {message}"),
        }
    }
}

/// What to do after fetching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Run `git merge --ff-only @{u}`, which will pull this many commits.
    FastForward {
        /// Commits behind the upstream.
        commits: u32,
    },
    /// Nothing more to run; this is the outcome.
    Done(SyncOutcome),
}

/// Decides the next step. Precedence: dirty, then detached, then no
/// upstream, then `behind == 0` (up to date), then `ahead > 0` (diverged),
/// else fast-forward.
#[must_use]
pub fn classify(facts: &RepoFacts) -> Plan {
    let (dirty, upstream) = match *facts {
        RepoFacts::Detached { dirty } => (dirty, None),
        RepoFacts::Branch { dirty, upstream } => (dirty, Some(upstream)),
    };
    let fetched = |reason| Plan::Done(SyncOutcome::Fetched(reason));
    if dirty {
        return fetched(FetchedReason::DirtyWorktree);
    }
    match upstream {
        None => fetched(FetchedReason::DetachedHead),
        Some(Upstream::None) => fetched(FetchedReason::NoUpstream),
        Some(Upstream::Tracking { behind: 0, .. }) => Plan::Done(SyncOutcome::UpToDate),
        Some(Upstream::Tracking { ahead, .. }) if ahead > 0 => fetched(FetchedReason::Diverged),
        Some(Upstream::Tracking { behind, .. }) => Plan::FastForward { commits: behind },
    }
}

/// One repo for [`sync_all`].
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

/// A sync failure.
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    /// Some repos ended in `SyncOutcome::Error`.
    #[error("{} of {total} repositories failed to sync", .failed.len())]
    SomeFailed {
        /// Paths relative to `work_dir`, sorted.
        failed: Vec<PathBuf>,
        /// Repos attempted.
        total: usize,
    },
    /// `pull` on a repo that has not been cloned.
    #[error("{repo} is not cloned (run 'work clone {repo}')")]
    NotCloned {
        /// The repo as shown to the user, e.g. `acme/foo`.
        repo: String,
    },
    /// `pull` ended in an error outcome.
    #[error("could not pull {repo}: {detail}")]
    PullFailed {
        /// The repo as shown to the user, e.g. `acme/foo`.
        repo: String,
        /// The outcome's message.
        detail: String,
    },
}

/// Fetches `repo` and fast-forwards it when safe, reporting each step to
/// `on_step`; every failure becomes `SyncOutcome::Error`.
#[must_use]
pub fn pull_one(repo: &Path, domain: &Domain, on_step: &dyn Fn(Step)) -> SyncOutcome {
    let fail = |err: &GitError| SyncOutcome::Error(crate::text::one_line(&err.to_string()));
    on_step(Step::Fetching);
    if let Err(err) = git::fetch(repo, domain) {
        return fail(&err);
    }
    let facts = match git::facts(repo) {
        Ok(facts) => facts,
        Err(err) => return fail(&err),
    };
    match classify(&facts) {
        Plan::Done(outcome) => outcome,
        Plan::FastForward { commits } => {
            on_step(Step::FastForwarding);
            match git::fast_forward(repo) {
                Ok(()) => SyncOutcome::Pulled { commits },
                Err(err) => fail(&err),
            }
        }
    }
}

/// The `SomeFailed` error for `reports`, or `None` when no repo errored.
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

/// Runs [`pull_one`] over `repos` with `jobs` workers; results are sorted by path.
#[must_use]
pub fn sync_all(
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
        let outcome = pull_one(&path, target.domain, &on_step);
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

    use super::*;

    fn branch(dirty: bool, upstream: Upstream) -> RepoFacts {
        RepoFacts::Branch { dirty, upstream }
    }

    fn tracking(ahead: u32, behind: u32) -> Upstream {
        Upstream::Tracking { ahead, behind }
    }

    #[test]
    fn classify_truth_table() {
        let fetched = |r| Plan::Done(SyncOutcome::Fetched(r));
        let cases = [
            (
                branch(true, tracking(0, 3)),
                fetched(FetchedReason::DirtyWorktree),
            ),
            (
                branch(true, Upstream::None),
                fetched(FetchedReason::DirtyWorktree),
            ),
            (
                RepoFacts::Detached { dirty: true },
                fetched(FetchedReason::DirtyWorktree),
            ),
            (
                RepoFacts::Detached { dirty: false },
                fetched(FetchedReason::DetachedHead),
            ),
            (
                branch(false, Upstream::None),
                fetched(FetchedReason::NoUpstream),
            ),
            (
                branch(false, tracking(0, 0)),
                Plan::Done(SyncOutcome::UpToDate),
            ),
            (
                branch(false, tracking(2, 0)),
                Plan::Done(SyncOutcome::UpToDate),
            ),
            (
                branch(false, tracking(1, 4)),
                fetched(FetchedReason::Diverged),
            ),
            (
                branch(false, tracking(0, 4)),
                Plan::FastForward { commits: 4 },
            ),
        ];
        for (facts, expected) in cases {
            assert_eq!(classify(&facts), expected, "{facts:?}");
        }
    }

    #[test]
    fn fast_forward_carries_behind_count() {
        assert_matches!(
            classify(&branch(false, tracking(0, 1))),
            Plan::FastForward { commits: 1 }
        );
    }
}
