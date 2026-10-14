//! The `git` subprocess seam, its auth environment, and pure parsers for its
//! output.
//!
//! Every captured git child runs with `LC_ALL=C`, `GIT_TERMINAL_PROMPT=0`,
//! `GIT_OPTIONAL_LOCKS=0` and `protocol.ext.allow=never` (see
//! [`auth_env::GitEnv`]); its stderr goes through `proc::one_line` and
//! `GitAuth::redact` before it can reach an error.

pub mod auth_env;
pub mod runner;
pub mod status;

pub use runner::ProcessGit;

use std::fmt::Debug;
use std::path::Path;

use crate::auth::GitAuth;
use crate::core::sync_outcome::RepoFacts;
use crate::domain::CloneUrl;

/// What `work` asks of git. `Sync` because sync workers share one
/// `&dyn Git` across scoped threads.
pub trait Git: Sync + Debug {
    /// `git clone <url> <dest>`; `dest`'s parent already exists.
    ///
    /// # Errors
    ///
    /// Any [`GitError`]; a failed clone leaves no partial `dest` behind.
    fn clone_repo(
        &self,
        url: &CloneUrl,
        dest: &Path,
        auth: Option<&GitAuth<'_>>,
    ) -> Result<(), GitError>;

    /// `git fetch --quiet` in `repo` (no remote named, so git uses the
    /// branch's own remote, falling back to `origin`).
    ///
    /// # Errors
    ///
    /// Any [`GitError`].
    fn fetch(&self, repo: &Path, auth: Option<&GitAuth<'_>>) -> Result<(), GitError>;

    /// One `git status --porcelain=v2 --branch --untracked-files=normal`,
    /// parsed by [`status::parse_v2`].
    ///
    /// # Errors
    ///
    /// Any [`GitError`], including [`GitError::Status`] for unparsable
    /// output.
    fn facts(&self, repo: &Path) -> Result<RepoFacts, GitError>;

    /// `git merge --ff-only --quiet @{u}`.
    ///
    /// # Errors
    ///
    /// Any [`GitError`].
    fn fast_forward(&self, repo: &Path) -> Result<(), GitError>;
}

/// A failure running git. Every `detail` is one line and already redacted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GitError {
    /// `git` is not on `PATH`.
    #[error("git is required (install git and make sure it is on PATH)")]
    NotInstalled,
    /// The installed git cannot take config from the environment.
    #[error("git {version} is too old (2.31 or later is required)")]
    TooOld {
        /// The version git reported.
        version: String,
    },
    /// git could not be started for another reason.
    #[error("could not run git: {detail}")]
    Spawn {
        /// The OS error, one line.
        detail: String,
    },
    /// `git <action>` ran and failed.
    #[error("git {action} failed: {detail}")]
    Failed {
        /// `clone`, `fetch`, `status`, or `merge`.
        action: &'static str,
        /// git's stderr, reduced to one line and redacted.
        detail: String,
    },
    /// `git status` output did not have the expected shape.
    #[error("unexpected git status output ({detail})")]
    Status {
        /// What was unexpected.
        detail: String,
    },
}
