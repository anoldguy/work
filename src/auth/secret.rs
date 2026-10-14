//! [`SecretToken`], the git auth header, and redaction.

use std::fmt;
use std::marker::PhantomData;

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

/// The `GITHUB_TOKEN` environment variable, as captured at startup.
///
/// An unusable value is its own state so it cannot be mistaken for "unset"
/// and silently replaced by a `gh` token.
#[derive(Debug, Default)]
pub enum EnvToken {
    /// Unset or empty.
    #[default]
    Unset,
    /// Set to a usable token; answers for every domain.
    Valid(SecretToken),
    /// Set but unusable (inner whitespace, control characters, not UTF-8).
    Invalid,
}

/// HTTP auth for one host's git traffic.
///
/// Passed to git through `GIT_CONFIG_*` environment variables as
/// `http.https://<host>/.extraHeader = Authorization: Basic <header_b64>`,
/// which keeps it out of argv and off other hosts. It borrows the raw token
/// so [`GitAuth::redact`] can scrub both forms from git's stderr without the
/// secret ever leaving this type.
pub struct GitAuth<'t> {
    host: Domain,
    header_b64: SecretToken,
    /// Redaction forms, longest first, computed once.
    forms: Vec<String>,
    /// Ties the auth to the token it was derived from.
    token: PhantomData<&'t SecretToken>,
}

impl fmt::Debug for GitAuth<'_> {
    /// Prints the host only: `forms` holds the raw token and its base64s.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitAuth")
            .field("host", &self.host)
            .finish_non_exhaustive()
    }
}

impl<'t> GitAuth<'t> {
    /// Builds the header for `token` on `host`:
    /// `base64("x-access-token:" + token)`.
    #[must_use]
    pub fn new(host: Domain, token: &'t SecretToken) -> Self {
        let credential = format!("{BASIC_USER}:{}", token.expose());
        let header_b64 = SecretToken(base64_encode(credential.as_bytes()));
        let forms = redaction_forms(&[token]);
        Self {
            host,
            header_b64,
            forms,
            token: PhantomData,
        }
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
        apply_redaction(line, &self.forms)
    }
}

/// Replaces every occurrence of each secret in `line` with `***`, in both
/// its raw form and its `GitAuth` base64 form (`base64("x-access-token:" +
/// secret)`).
#[must_use]
pub fn redact(line: &str, secrets: &[&SecretToken]) -> String {
    apply_redaction(line, &redaction_forms(secrets))
}

/// Every form of each secret to scrub, longest first and deduplicated.
fn redaction_forms(secrets: &[&SecretToken]) -> Vec<String> {
    let mut forms: Vec<String> = secrets
        .iter()
        .flat_map(|secret| {
            let raw = secret.expose();
            [
                base64_encode(format!("{BASIC_USER}:{raw}").as_bytes()),
                base64_encode(raw.as_bytes()),
                raw.to_owned(),
            ]
        })
        .collect();
    // Longest first so a longer form is never half-eaten by a shorter one.
    forms.sort_by_key(|form| std::cmp::Reverse(form.len()));
    forms.dedup();
    forms
}

fn apply_redaction(line: &str, forms: &[String]) -> String {
    forms.iter().fold(line.to_owned(), |text, form| {
        text.replace(form.as_str(), "***")
    })
}

/// The basic-auth user name GitHub accepts for token authentication.
const BASIC_USER: &str = "x-access-token";

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard RFC 4648 base64 with padding.
fn base64_encode(input: &[u8]) -> String {
    let sextet = |n: u32, shift: u32| char::from(BASE64_ALPHABET[(n >> shift & 0x3f) as usize]);
    input
        .chunks(3)
        .flat_map(|chunk| {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0u32, |acc, (i, &b)| acc | u32::from(b) << (16 - 8 * i));
            [
                Some(sextet(n, 18)),
                Some(sextet(n, 12)),
                Some(if chunk.len() > 1 { sextet(n, 6) } else { '=' }),
                Some(if chunk.len() > 2 { sextet(n, 0) } else { '=' }),
            ]
        })
        .flatten()
        .collect()
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use proptest::prelude::*;

    use super::{EnvToken, GitAuth, SecretToken, base64_encode, redact};
    use crate::domain::Domain;
    use crate::env::EnvSnapshot;

    #[test]
    fn base64_matches_rfc4648_vectors() {
        for (plain, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64_encode(plain.as_bytes()), encoded);
        }
    }

    #[test]
    fn git_auth_header_is_basic_x_access_token() {
        let token = SecretToken::new("gho_abc").expect("valid");
        let host = "github.com".parse::<Domain>().expect("valid domain");
        let auth = GitAuth::new(host, &token);
        assert_eq!(
            auth.header_b64().expose(),
            base64_encode(b"x-access-token:gho_abc")
        );
        assert_eq!(auth.host().as_str(), "github.com");
    }

    #[test]
    fn redact_removes_raw_and_base64_forms() {
        let token = SecretToken::new("gho_abc123").expect("valid");
        let host = "github.com".parse::<Domain>().expect("valid domain");
        let auth = GitAuth::new(host, &token);
        let line = format!(
            "fatal: gho_abc123 Authorization: Basic {} alt {}",
            auth.header_b64().expose(),
            base64_encode(b"gho_abc123"),
        );
        let out = auth.redact(&line);
        assert_eq!(out, "fatal: *** Authorization: Basic *** alt ***");
        assert_eq!(redact(&line, &[&token]), out);
    }

    #[test]
    fn env_snapshot_debug_hides_the_token() {
        let env = EnvSnapshot {
            github_token: SecretToken::new("ghp_topsecretvalue")
                .map_or(EnvToken::Invalid, EnvToken::Valid),
            ..EnvSnapshot::default()
        };
        assert_matches!(env.github_token, EnvToken::Valid(_));
        assert!(!format!("{env:?}").contains("topsecretvalue"));
    }

    #[test]
    fn no_token_holder_debug_shows_the_token_or_its_base64() {
        use crate::auth::Tokens;
        use crate::testing::FakeGh;

        let raw = "gho_PROBESECRET123";
        let host = "github.com".parse::<Domain>().expect("valid domain");
        let token = SecretToken::new(raw).expect("valid");
        let auth = GitAuth::new(host.clone(), &token);
        let gh = FakeGh::new().with_token(host.clone(), raw);
        let env_token = EnvToken::Valid(SecretToken::new(raw).expect("valid"));
        let tokens = Tokens::resolve(&gh, &EnvToken::Unset, std::slice::from_ref(&host));
        let tokens_env = Tokens::resolve(&gh, &env_token, std::slice::from_ref(&host));
        let snapshot = EnvSnapshot {
            github_token: SecretToken::new(raw).map_or(EnvToken::Invalid, EnvToken::Valid),
            ..EnvSnapshot::default()
        };
        let shown = [
            format!("{token:?}"),
            format!("{auth:?}"),
            format!("{tokens:?}"),
            format!("{tokens_env:?}"),
            format!("{gh:?}"),
            format!("{snapshot:?}"),
            format!("{env_token:?}"),
        ];
        let b64 = [
            base64_encode(raw.as_bytes()),
            base64_encode(format!("x-access-token:{raw}").as_bytes()),
        ];
        for text in &shown {
            assert!(!text.contains("PROBESECRET"), "{text}");
            assert!(b64.iter().all(|b| !text.contains(b.as_str())), "{text}");
        }
    }

    proptest! {
        #[test]
        fn redacted_output_never_contains_the_token(
            raw in "[A-Za-z0-9_]{6,40}",
            before in ".{0,40}",
            after in ".{0,40}",
            use_b64 in any::<bool>(),
        ) {
            let token = SecretToken::new(&raw).expect("valid token");
            let host = "github.com".parse::<Domain>().expect("valid domain");
            let auth = GitAuth::new(host, &token);
            let middle = if use_b64 {
                auth.header_b64().expose().to_owned()
            } else {
                raw.clone()
            };
            let out = auth.redact(&format!("{before}{raw} {middle}{after}"));
            prop_assert!(!out.contains(&raw));
            prop_assert!(!out.contains(auth.header_b64().expose()));
        }
    }

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
