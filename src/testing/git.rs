//! [`FakeGit`]: a scripted `Git` (filled in by W2.7).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::auth::GitAuth;
use crate::core::sync_outcome::RepoFacts;
use crate::domain::{CloneUrl, Domain};
use crate::git::{Git, GitError};

/// Which `Git` method was called.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GitOp {
    /// `clone_repo`.
    Clone,
    /// `fetch`.
    Fetch,
    /// `facts`.
    Facts,
    /// `fast_forward`.
    FastForward,
    /// `origin_url`.
    OriginUrl,
}

/// One recorded call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCall {
    /// The method.
    pub op: GitOp,
    /// The repo (for `Clone`, the destination).
    pub path: PathBuf,
    /// The host of the `GitAuth` passed, if any.
    pub auth_host: Option<Domain>,
}

/// How one repo behaves.
#[derive(Debug, Clone)]
pub struct RepoScript {
    /// What `facts` returns.
    pub facts: RepoFacts,
    /// What `origin_url` returns.
    pub origin: Option<String>,
    /// The one operation that fails, and how.
    pub fail: Option<(GitOp, GitError)>,
}

/// A `Git` that answers from per-path scripts, records every call, and
/// tracks the most calls in flight at once (for pool-limit tests).
///
/// `clone_repo` creates `dest/.git` on disk (tests pass a tempdir), so
/// "already cloned" checks see the result. Unscripted paths behave as a
/// clean branch that is up to date with no origin.
#[derive(Debug, Default)]
pub struct FakeGit {
    repos: BTreeMap<PathBuf, RepoScript>,
    log: Mutex<Vec<GitCall>>,
    in_flight: Mutex<(usize, usize)>,
}

impl FakeGit {
    /// A fake with no scripted repos.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Scripts the repo at `path`.
    #[must_use]
    pub fn with_repo(self, path: &Path, script: RepoScript) -> Self {
        let _ = (self, path, script);
        unimplemented!("W2.7")
    }

    /// Every call so far, in order.
    #[must_use]
    pub fn calls(&self) -> Vec<GitCall> {
        let _ = (&self.repos, &self.log);
        unimplemented!("W2.7")
    }

    /// The most calls that were running at the same time.
    #[must_use]
    pub fn max_in_flight(&self) -> usize {
        let _ = &self.in_flight;
        unimplemented!("W2.7")
    }
}

impl Git for FakeGit {
    fn clone_repo(
        &self,
        url: &CloneUrl,
        dest: &Path,
        auth: Option<&GitAuth<'_>>,
    ) -> Result<(), GitError> {
        let _ = (self, url, dest, auth);
        unimplemented!("W2.7")
    }

    fn fetch(&self, repo: &Path, auth: Option<&GitAuth<'_>>) -> Result<(), GitError> {
        let _ = (self, repo, auth);
        unimplemented!("W2.7")
    }

    fn facts(&self, repo: &Path) -> Result<RepoFacts, GitError> {
        let _ = (self, repo);
        unimplemented!("W2.7")
    }

    fn fast_forward(&self, repo: &Path) -> Result<(), GitError> {
        let _ = (self, repo);
        unimplemented!("W2.7")
    }

    fn origin_url(&self, repo: &Path) -> Result<Option<String>, GitError> {
        let _ = (self, repo);
        unimplemented!("W2.7")
    }
}
