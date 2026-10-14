//! [`FakeGh`]: a scripted `Gh` (filled in by W2.6).

use std::collections::BTreeMap;
use std::sync::Mutex;

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
#[derive(Debug, Default)]
pub struct FakeGh {
    tokens: Mutex<BTreeMap<Domain, Result<String, GhError>>>,
    after_login: BTreeMap<Domain, String>,
    log: Mutex<Vec<GhCall>>,
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
        let _ = (self, host, token);
        unimplemented!("W2.6")
    }

    /// `auth_token(host)` fails with `err`.
    #[must_use]
    pub fn with_error(self, host: Domain, err: GhError) -> Self {
        let _ = (self, host, err);
        unimplemented!("W2.6")
    }

    /// `auth_token(host)` returns `token` only after `login(host)`.
    #[must_use]
    pub fn with_token_after_login(self, host: Domain, token: &str) -> Self {
        let _ = (self, host, token);
        unimplemented!("W2.6")
    }

    /// Every call so far, in order.
    #[must_use]
    pub fn calls(&self) -> Vec<GhCall> {
        let _ = (&self.tokens, &self.after_login, &self.log);
        unimplemented!("W2.6")
    }
}

impl Gh for FakeGh {
    fn auth_token(&self, host: &Domain) -> Result<Option<SecretToken>, GhError> {
        let _ = (self, host);
        unimplemented!("W2.6")
    }

    fn login(&self, host: &Domain) -> Result<(), GhError> {
        let _ = (self, host);
        unimplemented!("W2.6")
    }

    fn logout(&self, host: &Domain) -> Result<(), GhError> {
        let _ = (self, host);
        unimplemented!("W2.6")
    }
}
