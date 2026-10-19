//! `adopted.json`: readable repositories that no listing returns, kept once
//! resolution finds them on GitHub.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::domain::{Domain, Repo, Slug};
use crate::state_file::{Problem, StateError, StateFile};

/// File name inside the cache directory.
pub const ADOPTED_FILE: &str = "adopted.json";

/// The only version this build reads or writes.
pub const ADOPTED_VERSION: u32 = 1;

/// `adopted.json`.
pub static ADOPTED: StateFile = StateFile {
    name: ADOPTED_FILE,
    version: ADOPTED_VERSION,
    noun: "adoption",
    fresh: "an empty list",
};

/// The contents of `adopted.json` minus its version.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Adopted {
    /// Every adopted repo, sorted by slug, one per slug ignoring case.
    #[serde(default)]
    pub repos: Vec<Repo>,
}

impl Adopted {
    /// Adds `repo` unless a repo with the same slug (ignoring case) is
    /// already there. Returns whether it was added.
    pub fn add(&mut self, repo: Repo) -> bool {
        if self.repos.iter().any(|r| same_slug(&r.slug, &repo.slug)) {
            return false;
        }
        self.repos.push(repo);
        self.repos.sort_by(|a, b| a.slug.cmp(&b.slug));
        true
    }

    /// Removes the repo with `slug` (ignoring case). Returns whether one was
    /// there.
    pub fn remove(&mut self, slug: &Slug) -> bool {
        let before = self.repos.len();
        self.repos.retain(|r| !same_slug(&r.slug, slug));
        self.repos.len() != before
    }
}

/// Whether two slugs name the same repository: GitHub compares owner and
/// name ignoring ASCII case, and domains are already lowercase.
#[must_use]
pub fn same_slug(a: &Slug, b: &Slug) -> bool {
    a.domain() == b.domain()
        && a.org().as_str().eq_ignore_ascii_case(b.org().as_str())
        && a.name().as_str().eq_ignore_ascii_case(b.name().as_str())
}

/// The adopted repos in `dir`, and the problem to warn about, if any.
#[must_use]
pub fn load(dir: &Path) -> (Adopted, Option<Problem>) {
    ADOPTED.load(dir)
}

/// Appends to `known` every repo of `adopted` on one of `domains` that
/// `known` does not already hold.
pub fn extend_known(known: &mut Vec<Repo>, adopted: Adopted, domains: &[Domain]) {
    for repo in adopted.repos {
        let listed = known.iter().any(|r| same_slug(&r.slug, &repo.slug));
        if !listed && domains.contains(repo.slug.domain()) {
            known.push(repo);
        }
    }
}

/// Applies `f` to the adopted repos in `dir` under the lock (see
/// [`StateFile::update`]).
///
/// # Errors
///
/// As for [`StateFile::update`].
pub fn update<R>(
    dir: &Path,
    f: impl FnOnce(&mut Adopted) -> R,
) -> Result<(R, Option<Problem>), StateError> {
    ADOPTED.update(dir, f)
}
