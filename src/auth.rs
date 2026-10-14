//! Tokens: the secret type, per-domain resolution, and git auth headers.

pub mod secret;
pub mod tokens;

pub use secret::{GitAuth, SecretToken, redact};
pub use tokens::{TokenSource, Tokens};

use crate::domain::Domain;
use crate::gh::GhError;

/// Why no token is available for a domain.
///
/// `Clone`, because [`Tokens`] keeps each domain's failure and hands callers
/// their own copy to propagate.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    /// `GITHUB_TOKEN` is unset and `gh` has no token for the host.
    #[error("no GitHub token for {domain} (run 'gh auth login --hostname {domain}')")]
    NoToken {
        /// The host that has no token.
        domain: Domain,
    },
    /// `GITHUB_TOKEN` is set but cannot be used as a token.
    #[error(
        "GITHUB_TOKEN is set but invalid (it must not contain whitespace or control characters)"
    )]
    InvalidEnvToken,
    /// Asking `gh` failed.
    #[error(transparent)]
    Gh(#[from] GhError),
}
