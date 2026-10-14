//! [`FakeGit`]: a scripted `Git`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use crate::auth::GitAuth;
use crate::core::sync_outcome::{RepoFacts, Upstream};
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

/// What an unscripted repo reports: a clean branch that is up to date.
const UNSCRIPTED_FACTS: RepoFacts = RepoFacts::Branch {
    dirty: false,
    upstream: Upstream::Tracking {
        ahead: 0,
        behind: 0,
    },
};

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
    cloned: Mutex<Vec<(PathBuf, CloneUrl)>>,
    in_flight: Mutex<(usize, usize)>,
}

/// Decrements the in-flight count when a call ends.
struct InFlight<'a>(&'a Mutex<(usize, usize)>);

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        let mut counts = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        counts.0 = counts.0.saturating_sub(1);
    }
}

impl FakeGit {
    /// A fake with no scripted repos.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Scripts the repo at `path`.
    #[must_use]
    pub fn with_repo(mut self, path: &Path, script: RepoScript) -> Self {
        self.repos.insert(path.to_path_buf(), script);
        self
    }

    /// Every call so far, in order.
    #[must_use]
    pub fn calls(&self) -> Vec<GitCall> {
        self.log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Every `clone_repo` call as `(dest, url)`, in order, including failed
    /// ones.
    #[must_use]
    pub fn cloned_urls(&self) -> Vec<(PathBuf, CloneUrl)> {
        self.cloned
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The most calls that were running at the same time.
    #[must_use]
    pub fn max_in_flight(&self) -> usize {
        self.in_flight
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .1
    }

    /// Records the call, marks it in flight, and returns the scripted
    /// failure for `op` (if any) with the guard that ends the call.
    fn begin(
        &self,
        op: GitOp,
        path: &Path,
        auth: Option<&GitAuth<'_>>,
    ) -> (InFlight<'_>, Result<Option<&RepoScript>, GitError>) {
        {
            let mut counts = self
                .in_flight
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            counts.0 = counts.0.saturating_add(1);
            counts.1 = counts.1.max(counts.0);
        }
        self.log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(GitCall {
                op,
                path: path.to_path_buf(),
                auth_host: auth.map(|a| a.host().clone()),
            });
        let script = self.repos.get(path);
        let result = match script.and_then(|s| s.fail.as_ref()) {
            Some((fail_op, err)) if *fail_op == op => Err(err.clone()),
            _ => Ok(script),
        };
        (InFlight(&self.in_flight), result)
    }
}

impl Git for FakeGit {
    fn clone_repo(
        &self,
        url: &CloneUrl,
        dest: &Path,
        auth: Option<&GitAuth<'_>>,
    ) -> Result<(), GitError> {
        let (_guard, result) = self.begin(GitOp::Clone, dest, auth);
        self.cloned
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((dest.to_path_buf(), url.clone()));
        result?;
        std::fs::create_dir_all(dest.join(".git")).map_err(|e| GitError::Failed {
            action: "clone",
            detail: e.to_string(),
        })
    }

    fn fetch(&self, repo: &Path, auth: Option<&GitAuth<'_>>) -> Result<(), GitError> {
        let (_guard, result) = self.begin(GitOp::Fetch, repo, auth);
        result.map(drop)
    }

    fn facts(&self, repo: &Path) -> Result<RepoFacts, GitError> {
        let (_guard, result) = self.begin(GitOp::Facts, repo, None);
        Ok(result?.map_or(UNSCRIPTED_FACTS, |s| s.facts))
    }

    fn fast_forward(&self, repo: &Path) -> Result<(), GitError> {
        let (_guard, result) = self.begin(GitOp::FastForward, repo, None);
        result.map(drop)
    }

    fn origin_url(&self, repo: &Path) -> Result<Option<String>, GitError> {
        let (_guard, result) = self.begin(GitOp::OriginUrl, repo, None);
        Ok(result?.and_then(|s| s.origin.clone()))
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::auth::SecretToken;

    fn failing(op: GitOp) -> RepoScript {
        RepoScript {
            facts: RepoFacts::Detached { dirty: true },
            origin: Some("https://github.com/acme/foo.git".to_owned()),
            fail: Some((
                op,
                GitError::Failed {
                    action: "fetch",
                    detail: "boom".to_owned(),
                },
            )),
        }
    }

    #[test]
    fn scripted_and_unscripted_answers() {
        let scripted = Path::new("/w/acme/foo");
        let git = FakeGit::new().with_repo(scripted, failing(GitOp::Fetch));
        assert_matches!(git.fetch(scripted, None), Err(GitError::Failed { .. }));
        assert_eq!(
            git.facts(scripted).expect("facts"),
            RepoFacts::Detached { dirty: true }
        );
        assert_eq!(
            git.origin_url(scripted).expect("origin").as_deref(),
            Some("https://github.com/acme/foo.git")
        );
        let other = Path::new("/w/acme/bar");
        assert_eq!(git.facts(other).expect("facts"), UNSCRIPTED_FACTS);
        assert_eq!(git.origin_url(other).expect("origin"), None);
        assert_matches!(git.fast_forward(other), Ok(()));
    }

    #[test]
    fn records_calls_with_auth_host_and_in_flight() {
        let token = SecretToken::new("gho_fake").expect("token");
        let host: Domain = "github.com".parse().expect("domain");
        let auth = GitAuth::new(host.clone(), &token);
        let git = FakeGit::new();
        let repo = Path::new("/w/a");
        git.fetch(repo, Some(&auth)).expect("fetch");
        git.facts(repo).expect("facts");
        assert_eq!(
            git.calls(),
            [
                GitCall {
                    op: GitOp::Fetch,
                    path: repo.to_path_buf(),
                    auth_host: Some(host),
                },
                GitCall {
                    op: GitOp::Facts,
                    path: repo.to_path_buf(),
                    auth_host: None,
                },
            ]
        );
        assert_eq!(git.max_in_flight(), 1);
    }

    #[test]
    fn clone_creates_git_dir_unless_scripted_to_fail() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let url = CloneUrl::try_from("https://github.com/acme/foo.git".to_owned()).expect("url");
        let ok = tmp.path().join("ok");
        let bad = tmp.path().join("bad");
        let git = FakeGit::new().with_repo(&bad, failing(GitOp::Clone));
        git.clone_repo(&url, &ok, None).expect("clone");
        assert!(ok.join(".git").is_dir());
        assert_matches!(
            git.clone_repo(&url, &bad, None),
            Err(GitError::Failed { .. })
        );
        assert!(!bad.exists());
        assert_eq!(git.cloned_urls(), [(ok, url.clone()), (bad, url)]);
    }
}
