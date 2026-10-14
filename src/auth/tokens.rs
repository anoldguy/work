//! [`Tokens`]: an immutable per-domain token map, built once per need.

use std::collections::BTreeMap;

use crate::auth::{AuthError, EnvToken, SecretToken};
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
    env: &'e EnvToken,
    by_domain: BTreeMap<Domain, Result<SecretToken, AuthError>>,
}

impl<'e> Tokens<'e> {
    /// Resolves a token for each of `domains`: `env` when present (without
    /// running `gh`), otherwise `gh.auth_token(domain)`.
    #[must_use]
    pub fn resolve(gh: &dyn Gh, env: &'e EnvToken, domains: &[Domain]) -> Self {
        let by_domain = if matches!(env, EnvToken::Unset) {
            domains
                .iter()
                .map(|d| {
                    let result = match gh.auth_token(d) {
                        Ok(Some(token)) => Ok(token),
                        Ok(None) => Err(AuthError::NoToken { domain: d.clone() }),
                        Err(err) => Err(AuthError::Gh(err)),
                    };
                    (d.clone(), result)
                })
                .collect()
        } else {
            BTreeMap::new()
        };
        Self { env, by_domain }
    }

    /// The token for `d` and where it came from.
    ///
    /// # Errors
    ///
    /// The domain's resolution failure, or [`AuthError::NoToken`] for a
    /// domain that was not part of [`Tokens::resolve`] (unless the env token
    /// is set, which covers every domain).
    pub fn get(&self, d: &Domain) -> Result<(&SecretToken, TokenSource), AuthError> {
        match self.env {
            EnvToken::Valid(token) => return Ok((token, TokenSource::EnvGithubToken)),
            EnvToken::Invalid => return Err(AuthError::InvalidEnvToken),
            EnvToken::Unset => {}
        }
        match self.by_domain.get(d) {
            Some(Ok(token)) => Ok((token, TokenSource::GhCli)),
            Some(Err(err)) => Err(err.clone()),
            None => Err(AuthError::NoToken { domain: d.clone() }),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::gh::GhError;
    use crate::testing::FakeGh;

    fn dom(s: &str) -> Domain {
        s.parse().expect("valid domain")
    }

    const fn assert_sync<T: Sync>() {}

    #[test]
    fn tokens_are_sync() {
        assert_sync::<Tokens<'static>>();
    }

    #[test]
    fn env_token_wins_for_every_domain_and_gh_is_never_called() {
        let gh = FakeGh::new().with_token(dom("github.com"), "gho_gh");
        let env = EnvToken::Valid(SecretToken::new("ghp_env").expect("valid"));
        let tokens = Tokens::resolve(&gh, &env, &[dom("github.com")]);
        let (t, src) = tokens.get(&dom("github.com")).expect("token");
        assert_eq!(t.expose(), "ghp_env");
        assert_eq!(src, TokenSource::EnvGithubToken);
        assert_matches!(tokens.get(&dom("ghe.example.com")), Ok(_));
        assert_eq!(gh.calls().len(), 0);
    }

    #[test]
    fn gh_tokens_are_resolved_per_domain() {
        let gh = FakeGh::new().with_token(dom("github.com"), "gho_gh");
        let tokens = Tokens::resolve(
            &gh,
            &EnvToken::Unset,
            &[dom("github.com"), dom("ghe.example.com")],
        );
        let (t, src) = tokens.get(&dom("github.com")).expect("token");
        assert_eq!((t.expose(), src), ("gho_gh", TokenSource::GhCli));
        assert_matches!(
            tokens.get(&dom("ghe.example.com")),
            Err(AuthError::NoToken { .. })
        );
        assert_matches!(
            tokens.get(&dom("other.example.com")),
            Err(AuthError::NoToken { .. })
        );
        assert_eq!(gh.calls().len(), 2);
    }

    #[test]
    fn missing_gh_is_reported_and_hint_names_the_domain() {
        let gh = FakeGh::new().with_error(dom("github.com"), GhError::NotInstalled);
        let tokens = Tokens::resolve(&gh, &EnvToken::Unset, &[dom("github.com")]);
        let err = tokens.get(&dom("github.com")).unwrap_err();
        assert_matches!(err, AuthError::Gh(GhError::NotInstalled));
        assert!(err.to_string().contains("required"));
        let none = Tokens::resolve(&FakeGh::new(), &EnvToken::Unset, &[dom("github.com")]);
        let msg = none.get(&dom("github.com")).unwrap_err().to_string();
        assert!(msg.contains("gh auth login --hostname github.com"), "{msg}");
    }

    #[test]
    fn failures_are_not_memoised_across_resolves() {
        let gh = FakeGh::new().with_token_after_login(dom("github.com"), "gho_new");
        let before = Tokens::resolve(&gh, &EnvToken::Unset, &[dom("github.com")]);
        assert_matches!(before.get(&dom("github.com")), Err(_));
        gh.login(&dom("github.com")).expect("login");
        let after = Tokens::resolve(&gh, &EnvToken::Unset, &[dom("github.com")]);
        assert_matches!(after.get(&dom("github.com")), Ok(_));
    }

    #[test]
    fn debug_hides_the_token() {
        let gh = FakeGh::new().with_token(dom("github.com"), "gho_topsecret");
        let tokens = Tokens::resolve(&gh, &EnvToken::Unset, &[dom("github.com")]);
        assert!(!format!("{tokens:?}").contains("topsecret"));
    }

    #[test]
    fn invalid_env_token_is_an_error_and_never_falls_back_to_gh() {
        let gh = FakeGh::new().with_token(dom("github.com"), "gho_gh");
        let tokens = Tokens::resolve(&gh, &EnvToken::Invalid, &[dom("github.com")]);
        assert_matches!(
            tokens.get(&dom("github.com")),
            Err(AuthError::InvalidEnvToken)
        );
        assert_eq!(gh.calls().len(), 0);
    }

    #[test]
    fn source_labels() {
        assert_eq!(TokenSource::EnvGithubToken.label(), "GITHUB_TOKEN");
        assert_eq!(TokenSource::GhCli.label(), "gh");
    }
}
