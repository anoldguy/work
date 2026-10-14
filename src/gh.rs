//! The `gh` subprocess seam: token lookup, login, logout.

use std::fmt::Debug;

use crate::auth::SecretToken;
use crate::domain::Domain;

/// What `work` asks of the GitHub CLI. `Sync` so it can sit in `App` beside
/// the other seams; implemented by [`ProcessGh`] and by test fakes.
pub trait Gh: Sync + Debug {
    /// `gh auth token --hostname <host>`. `Ok(None)` when gh has no token
    /// for the host.
    ///
    /// # Errors
    ///
    /// [`GhError::NotInstalled`] when `gh` is not on `PATH`, or
    /// [`GhError::Failed`] for any other failure.
    fn auth_token(&self, host: &Domain) -> Result<Option<SecretToken>, GhError>;

    /// `gh auth login --hostname <host>` with stdio inherited (interactive).
    ///
    /// # Errors
    ///
    /// [`GhError::NotInstalled`], or [`GhError::Failed`] on a non-zero exit.
    fn login(&self, host: &Domain) -> Result<(), GhError>;

    /// `gh auth logout --hostname <host>` with stdio inherited.
    ///
    /// # Errors
    ///
    /// [`GhError::NotInstalled`], or [`GhError::Failed`] on a non-zero exit.
    fn logout(&self, host: &Domain) -> Result<(), GhError>;
}

/// The real `gh`, run as a child process found on `PATH`.
#[derive(Debug, Default)]
pub struct ProcessGh;

impl Gh for ProcessGh {
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

/// A failure running `gh`. `Clone` so it can live inside a cloned
/// `AuthError`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GhError {
    /// `gh` is not on `PATH`. Only possible when `GITHUB_TOKEN` is unset.
    #[error("GitHub CLI 'gh' is required (install from https://cli.github.com)")]
    NotInstalled,
    /// `gh` could not be started for another reason.
    #[error("could not run gh: {detail}")]
    Spawn {
        /// The OS error, one line.
        detail: String,
    },
    /// `gh auth <action>` ran and failed.
    #[error("gh auth {action} failed for {host}: {detail}")]
    Failed {
        /// `token`, `login`, or `logout`.
        action: &'static str,
        /// The host it ran against.
        host: Domain,
        /// gh's stderr through `proc::one_line` and `auth::redact`, or the
        /// exit status for inherited-stdio runs.
        detail: String,
    },
}
