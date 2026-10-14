//! The per-repo pipeline (fetch, facts, classify, fast-forward) and the
//! bounded `sync_all`.

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use crate::auth::{GitAuth, Tokens};
use crate::core::sync_outcome::SyncOutcome;
use crate::domain::Domain;
use crate::git::Git;
use crate::progress::{ProgressSink, Step};

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
    let _ = (git, repo, auth, on_step);
    unimplemented!("W3.4")
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
    let _ = (git, work_dir, repos, tokens, configured, jobs, sink);
    unimplemented!("W3.4")
}
