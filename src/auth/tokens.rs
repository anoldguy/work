//! [`Tokens`]: an immutable per-domain token map, built once per need.

use std::collections::BTreeMap;

use crate::auth::{AuthError, SecretToken};
use crate::domain::Domain;
use crate::gh::Gh;

/// Where a token came from, for `auth status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenSource {
    /// The `GITHUB_TOKEN` environment variable.
    EnvGithubToken,
    /// `gh auth token --hostname <domain>`.
    GhCli,
}

impl TokenSource {
    /// Short label for status output: `GITHUB_TOKEN` or `gh`.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::EnvGithubToken => "GITHUB_TOKEN",
            Self::GhCli => "gh",
        }
    }
}

/// Tokens for a set of domains, resolved up front on the main thread.
///
/// Immutable and `Sync`, so fetch and sync workers share it by reference.
/// When `GITHUB_TOKEN` is set it answers for every domain and `gh` is never
/// run. Failures are kept per domain but never memoised across builds: after
/// `gh auth login`, build a new `Tokens`.
#[derive(Debug)]
pub struct Tokens<'e> {
    env: Option<&'e SecretToken>,
    by_domain: BTreeMap<Domain, Result<SecretToken, AuthError>>,
}

impl<'e> Tokens<'e> {
    /// Resolves a token for each of `domains`: `env` when present (without
    /// running `gh`), otherwise `gh.auth_token(domain)`.
    #[must_use]
    pub fn resolve(gh: &dyn Gh, env: Option<&'e SecretToken>, domains: &[Domain]) -> Self {
        let _ = (gh, env, domains);
        unimplemented!("W2.6")
    }

    /// The token for `d` and where it came from.
    ///
    /// # Errors
    ///
    /// The domain's resolution failure, or [`AuthError::NoToken`] for a
    /// domain that was not part of [`Tokens::resolve`] (unless the env token
    /// is set, which covers every domain).
    pub fn get(&self, d: &Domain) -> Result<(&SecretToken, TokenSource), AuthError> {
        let _ = (self.env, &self.by_domain, d);
        unimplemented!("W2.6")
    }
}
