//! [`SecretToken`], the git auth header, and redaction.

use std::fmt;

use crate::domain::Domain;

/// A GitHub token (or derived header) that must never reach output.
///
/// `Debug` prints `SecretToken(***)`. There is deliberately no `Display`,
/// `Serialize`, or `Clone`: the only way to read the value is
/// [`SecretToken::expose`], at the point it is handed to a child process or
/// an HTTP header.
///
/// Invariant: non-empty, no whitespace or control characters (so it cannot
/// split a header or an environment value).
pub struct SecretToken(String);

impl SecretToken {
    /// Trims `raw` (for example `gh auth token` output with its trailing
    /// newline) and wraps it. Returns `None` when nothing is left or when
    /// whitespace or control characters remain inside.
    #[must_use]
    pub fn new(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        let clean =
            !trimmed.is_empty() && !trimmed.chars().any(|c| c.is_whitespace() || c.is_control());
        clean.then(|| Self(trimmed.to_owned()))
    }

    /// The secret value. Call this only where the value leaves the process
    /// (child environment, HTTP header) or for redaction.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretToken(***)")
    }
}

/// HTTP auth for one host's git traffic.
///
/// Passed to git through `GIT_CONFIG_*` environment variables as
/// `http.https://<host>/.extraHeader = Authorization: Basic <header_b64>`,
/// which keeps it out of argv and off other hosts. It borrows the raw token
/// so [`GitAuth::redact`] can scrub both forms from git's stderr without the
/// secret ever leaving this type.
#[derive(Debug)]
pub struct GitAuth<'t> {
    host: Domain,
    token: &'t SecretToken,
    header_b64: SecretToken,
}

impl<'t> GitAuth<'t> {
    /// Builds the header for `token` on `host`:
    /// `base64("x-access-token:" + token)`.
    #[must_use]
    pub fn new(host: Domain, token: &'t SecretToken) -> Self {
        let _ = (host, token);
        unimplemented!("W1.4")
    }

    /// The host the header is scoped to.
    #[must_use]
    pub const fn host(&self) -> &Domain {
        &self.host
    }

    /// The base64 credential (without the `Basic ` prefix).
    #[must_use]
    pub const fn header_b64(&self) -> &SecretToken {
        &self.header_b64
    }

    /// `line` with the raw token and its base64 header form replaced by
    /// `***`. Apply to every line of git output before it is shown.
    #[must_use]
    pub fn redact(&self, line: &str) -> String {
        redact(line, &[self.token])
    }
}

/// Replaces every occurrence of each secret in `line` with `***`, in both
/// its raw form and its `GitAuth` base64 form (`base64("x-access-token:" +
/// secret)`).
#[must_use]
pub fn redact(line: &str, secrets: &[&SecretToken]) -> String {
    let _ = (line, secrets);
    unimplemented!("W1.4")
}

#[cfg(test)]
mod tests {
    use super::SecretToken;

    #[test]
    fn new_trims_and_rejects_blank_or_split_tokens() {
        let token = SecretToken::new("  gho_abc123\n").map(|t| t.expose().to_owned());
        assert_eq!(token.as_deref(), Some("gho_abc123"));
        assert!(SecretToken::new("").is_none());
        assert!(SecretToken::new(" \n").is_none());
        assert!(SecretToken::new("gho_a b").is_none());
        assert!(SecretToken::new("gho_a\u{7}b").is_none());
    }

    #[test]
    fn debug_never_shows_the_value() {
        let token = SecretToken::new("gho_supersecret").expect("valid token");
        let shown = format!("{token:?} {:?}", Some(&token));
        assert!(!shown.contains("supersecret"), "{shown}");
        assert_eq!(format!("{token:?}"), "SecretToken(***)");
    }
}
