//! [`FakeGh`]: a scripted `Gh`.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::auth::SecretToken;
use crate::domain::Domain;
use crate::gh::{Gh, GhError};

/// One recorded call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhCall {
    /// `token`, `login`, or `logout`.
    pub action: &'static str,
    /// The host it was called for.
    pub host: Domain,
}

/// A `Gh` that answers `auth_token` from a per-host script and records every
/// call. Unscripted hosts have no token (`Ok(None)`). A token registered with
/// [`FakeGh::with_token_after_login`] becomes available once `login` has been
/// called for that host.
#[derive(Default)]
pub struct FakeGh {
    tokens: Mutex<BTreeMap<Domain, Result<String, GhError>>>,
    after_login: BTreeMap<Domain, String>,
    log: Mutex<Vec<GhCall>>,
}

impl std::fmt::Debug for FakeGh {
    /// Hosts and call count only: the scripted tokens stay out of `Debug`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeGh")
            .field("hosts", &lock(&self.tokens).keys().collect::<Vec<_>>())
            .field("calls", &lock(&self.log).len())
            .finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl FakeGh {
    /// A fake where no host has a token.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `auth_token(host)` returns `token`.
    #[must_use]
    pub fn with_token(self, host: Domain, token: &str) -> Self {
        lock(&self.tokens).insert(host, Ok(token.to_owned()));
        self
    }

    /// `auth_token(host)` fails with `err`.
    #[must_use]
    pub fn with_error(self, host: Domain, err: GhError) -> Self {
        lock(&self.tokens).insert(host, Err(err));
        self
    }

    /// `auth_token(host)` returns `token` only after `login(host)`.
    #[must_use]
    pub fn with_token_after_login(mut self, host: Domain, token: &str) -> Self {
        self.after_login.insert(host, token.to_owned());
        self
    }

    /// Every call so far, in order.
    #[must_use]
    pub fn calls(&self) -> Vec<GhCall> {
        lock(&self.log).clone()
    }

    fn record(&self, action: &'static str, host: &Domain) {
        lock(&self.log).push(GhCall {
            action,
            host: host.clone(),
        });
    }
}

impl Gh for FakeGh {
    fn auth_token(&self, host: &Domain) -> Result<Option<SecretToken>, GhError> {
        self.record("token", host);
        match lock(&self.tokens).get(host) {
            Some(Ok(token)) => Ok(SecretToken::new(token)),
            Some(Err(err)) => Err(err.clone()),
            None => Ok(None),
        }
    }

    fn login(&self, host: &Domain) -> Result<(), GhError> {
        self.record("login", host);
        if let Some(token) = self.after_login.get(host) {
            lock(&self.tokens).insert(host.clone(), Ok(token.clone()));
        }
        Ok(())
    }

    fn logout(&self, host: &Domain) -> Result<(), GhError> {
        self.record("logout", host);
        lock(&self.tokens).remove(host);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;

    fn host() -> Domain {
        "github.com".parse().expect("valid domain")
    }

    #[test]
    fn unscripted_host_has_no_token_and_calls_are_recorded() {
        let gh = FakeGh::new();
        assert_matches!(gh.auth_token(&host()), Ok(None));
        assert_eq!(
            gh.calls(),
            vec![GhCall {
                action: "token",
                host: host()
            }]
        );
    }

    #[test]
    fn token_appears_after_login_and_disappears_after_logout() {
        let gh = FakeGh::new().with_token_after_login(host(), "gho_x");
        assert_matches!(gh.auth_token(&host()), Ok(None));
        gh.login(&host()).expect("login");
        let token = gh.auth_token(&host()).expect("ok").expect("some");
        assert_eq!(token.expose(), "gho_x");
        gh.logout(&host()).expect("logout");
        assert_matches!(gh.auth_token(&host()), Ok(None));
    }

    #[test]
    fn scripted_error_is_returned() {
        let gh = FakeGh::new().with_error(host(), GhError::NotInstalled);
        assert_matches!(gh.auth_token(&host()), Err(GhError::NotInstalled));
    }
}
