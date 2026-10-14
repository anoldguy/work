//! [`ProcessGit`]: the real git, run as a child process.

use std::ffi::OsString;
use std::path::Path;

use crate::auth::GitAuth;
use crate::core::sync_outcome::RepoFacts;
use crate::domain::CloneUrl;
use crate::env::{EnvSnapshot, GitConfigCount};
use crate::git::{Git, GitError};

/// Runs `git` from `PATH` with the environment from `git::auth_env`.
#[derive(Debug)]
pub struct ProcessGit {
    config_count: GitConfigCount,
    trace_vars: Vec<OsString>,
}

impl ProcessGit {
    /// Copies what it needs from `env` (`GIT_CONFIG_COUNT` and the trace
    /// variable names to strip).
    #[must_use]
    pub fn new(env: &EnvSnapshot) -> Self {
        Self {
            config_count: env.git_config_count,
            trace_vars: env.git_trace_vars.clone(),
        }
    }
}

impl Git for ProcessGit {
    fn clone_repo(
        &self,
        url: &CloneUrl,
        dest: &Path,
        auth: Option<&GitAuth<'_>>,
    ) -> Result<(), GitError> {
        let _ = (self.config_count, &self.trace_vars, url, dest, auth);
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
