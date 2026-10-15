//! Fetching and fast-forwarding one repo, or many in parallel.

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use crate::core::sync_outcome::{Plan, SyncOutcome, classify};
use crate::domain::Domain;
use crate::git::{Git, GitError};
use crate::pool::run_bounded;
use crate::progress::{Noun, Step, Tasks, Verb};
use crate::sync::SyncError;

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

/// Fetches `repo` and fast-forwards it when safe, reporting each step to
/// `on_step`; every failure becomes `SyncOutcome::Error`.
#[must_use]
pub fn pull_one(
    git: &dyn Git,
    repo: &Path,
    domain: &Domain,
    on_step: &dyn Fn(Step),
) -> SyncOutcome {
    let fail = |err: &GitError| SyncOutcome::Error(crate::text::one_line(&err.to_string()));
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
