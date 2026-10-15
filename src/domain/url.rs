//! [`CloneUrl`] and [`HtmlUrl`]: URLs that are safe to hand to git or a
//! browser (pure).

use std::fmt;

use serde::{Deserialize, Serialize};

use super::NameError;

/// The parts of an absolute `scheme://authority...` URL.
struct Origin<'a> {
    scheme: &'a str,
    /// Host as written (IPv6 keeps its brackets), without user info or port.
    host: &'a str,
    has_user_info: bool,
    /// User info before the `@`, empty when absent.
    user: &'a str,
}

/// Splits `scheme://authority...`; `None` when there is no `://` or the
/// scheme or port is malformed.
fn split_origin(url: &str) -> Option<Origin<'_>> {
    let (scheme, after) = url.split_once("://")?;
    if scheme.is_empty()
        || !scheme
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+')
    {
        return None;
    }
    let end = after.find(['/', '\\', '?', '#']).unwrap_or(after.len());
    let authority = &after[..end];
    let (user, hostport) = authority.rsplit_once('@').unwrap_or(("", authority));
    let has_user_info = authority.contains('@');
    let (host, port) = if hostport.starts_with('[') {
        let (host, tail) = hostport.split_at(hostport.find(']')? + 1);
        if tail.is_empty() {
            (host, None)
        } else {
            (host, Some(tail.strip_prefix(':')?))
        }
    } else {
        match hostport.split_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (hostport, None),
        }
    };
    if port.is_some_and(|p| !p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    Some(Origin {
        scheme,
        host,
        has_user_info,
        user,
    })
}

/// True when `s` has a control or invisible format character, whitespace or
/// a backslash (browsers read `\` as `/` in https URLs) anywhere.
fn has_unsafe_chars(s: &str) -> bool {
    s.chars()
        .any(|c| crate::text::is_unsafe(c) || c.is_whitespace() || c == '\\')
}

/// True when `host` is a bracketed IPv6 literal (`[` hex digits, ':' and '.'
/// `]`) or contains no brackets at all.
fn host_brackets_ok(host: &str) -> bool {
    host.strip_prefix('[').map_or_else(
        || !host.contains([']', '[']),
        |inner| {
            inner.strip_suffix(']').is_some_and(|v6| {
                !v6.is_empty()
                    && v6
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.')
            })
        },
    )
}

/// A URL that is safe to pass to `git clone` as an argument.
///
/// Invariant: `https://`, `ssh://`, scp-like (`git@host:org/repo.git`), or
/// `file://` (local test fixtures). It never starts with `-` and is never an
/// `ext::` transport.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CloneUrl(String);

impl CloneUrl {
    /// The URL as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Why `s` is not an allowed clone URL, if it is not one.
fn clone_url_problem(s: &str) -> Option<&'static str> {
    if s.is_empty() {
        return Some("empty");
    }
    if s.starts_with('-') {
        return Some("cannot start with '-'");
    }
    if has_unsafe_chars(s) {
        return Some("whitespace and control characters are not allowed");
    }
    if let Some(o) = split_origin(s) {
        if o.user.starts_with('-') {
            return Some("user cannot start with '-'");
        }
        if !host_brackets_ok(o.host) {
            return Some("malformed host");
        }
        return match o.scheme {
            "https" | "ssh" if o.host.is_empty() => Some("missing host"),
            "https" | "ssh" if o.host.starts_with('-') => Some("host cannot start with '-'"),
            "https" | "ssh" | "file" => None,
            _ => Some("only https, ssh, scp-like and file URLs are allowed"),
        };
    }
    if s.contains("://") {
        return Some("malformed URL");
    }
    // scp-like: `[user@]host:path`, where host has no '/' (else it is a path).
    match s.split_once(':') {
        Some((userhost, path)) if !userhost.contains('/') && !path.is_empty() => {
            let host = userhost.rsplit_once('@').map_or(userhost, |(_, h)| h);
            if host.is_empty() || host.starts_with('-') || host.contains(['[', ']']) {
                Some("malformed scp-like URL")
            } else if host.eq_ignore_ascii_case("ext") || path.starts_with(':') {
                Some("the ext:: transport is not allowed")
            } else {
                None
            }
        }
        _ => Some("only https, ssh, scp-like and file URLs are allowed"),
    }
}

impl TryFrom<String> for CloneUrl {
    type Error = NameError;

    /// Validates a clone URL.
    ///
    /// # Errors
    ///
    /// [`NameError::CloneUrl`] for any other scheme, a leading `-`, or `ext::`.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match clone_url_problem(&value) {
            None => Ok(Self(value)),
            Some(reason) => Err(NameError::CloneUrl {
                given: value,
                reason,
            }),
        }
    }
}

impl From<CloneUrl> for String {
    fn from(value: CloneUrl) -> Self {
        value.0
    }
}

impl fmt::Display for CloneUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A repository's web page.
///
/// Invariant: `https://` with a non-empty host and no user info. The host is
/// not checked against any domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct HtmlUrl(String);

impl HtmlUrl {
    /// The URL as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for HtmlUrl {
    type Error = NameError;

    /// Validates a web URL.
    ///
    /// # Errors
    ///
    /// [`NameError::HtmlUrl`] for a non-https URL or one without a host.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let reason = if has_unsafe_chars(&value) {
            Some("whitespace and control characters are not allowed")
        } else {
            match split_origin(&value) {
                Some(o) if o.scheme == "https" => {
                    if o.has_user_info {
                        Some("user info is not allowed")
                    } else if o.host.is_empty() {
                        Some("missing host")
                    } else {
                        None
                    }
                }
                _ => Some("must be an https URL"),
            }
        };
        match reason {
            None => Ok(Self(value)),
            Some(reason) => Err(NameError::HtmlUrl {
                given: value,
                reason,
            }),
        }
    }
}

impl From<HtmlUrl> for String {
    fn from(value: HtmlUrl) -> Self {
        value.0
    }
}

impl fmt::Display for HtmlUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::{CloneUrl, HtmlUrl};
    use crate::domain::NameError;

    fn clone(s: &str) -> Result<CloneUrl, NameError> {
        CloneUrl::try_from(s.to_owned())
    }
    fn html(s: &str) -> Result<HtmlUrl, NameError> {
        HtmlUrl::try_from(s.to_owned())
    }

    #[test]
    fn clone_url_accepts_https_ssh_scp_and_file() {
        for ok in [
            "https://github.com/acme/x.git",
            "ssh://git@github.com/acme/x.git",
            "git@github.com:acme/x.git",
            "github.com:acme/x.git",
            "file:///tmp/fixture/x.git",
        ] {
            assert_eq!(clone(ok).unwrap().as_str(), ok);
        }
    }

    #[test]
    fn clone_url_rejects_dangerous_forms() {
        for bad in [
            "",
            "-oProxyCommand=evil",
            "--upload-pack=evil",
            "ext::sh -c evil",
            "ext::sh%20evil",
            "EXT::evil",
            "http://github.com/acme/x.git",
            "git://github.com/acme/x.git",
            "ftp://github.com/x",
            "/tmp/local/repo",
            "./repo",
            "https://",
            "ssh://-oProxyCommand=evil/x",
            "ssh://-oX@host/x",
            "ssh://git@-oX/x",
            "-oX@host:x",
            "git@-oevil:x",
            "git@-oevil:x",
            "https://github.com/a b",
            "https://github.com/a\nb",
        ] {
            assert_matches!(clone(bad), Err(NameError::CloneUrl { .. }), "{bad:?}");
        }
    }

    #[test]
    fn html_url_requires_https_and_host() {
        assert_eq!(
            html("https://github.com/acme/x").unwrap().as_str(),
            "https://github.com/acme/x"
        );
        for bad in [
            "",
            "http://github.com/x",
            "https://",
            "https:///x",
            "github.com/x",
            "javascript:alert(1)",
            "file:///x",
            "https://a b/",
        ] {
            assert_matches!(html(bad), Err(NameError::HtmlUrl { .. }), "{bad:?}");
        }
    }

    #[test]
    fn backslash_cannot_smuggle_a_different_host() {
        for bad in [
            "https://evil.com\\@github.com/x",
            "https://github.com\\@evil.com/x",
            "https://evil.com/\\x",
        ] {
            assert_matches!(html(bad), Err(NameError::HtmlUrl { .. }), "{bad:?}");
            assert_matches!(clone(bad), Err(NameError::CloneUrl { .. }), "{bad:?}");
        }
    }

    #[test]
    fn html_url_rejects_user_info() {
        assert_matches!(
            html("https://github.com@evil.com/x"),
            Err(NameError::HtmlUrl { .. })
        );
    }

    #[test]
    fn bracketed_hosts_must_be_ipv6_literals() {
        for bad in [
            "ssh://[-oProxyCommand=x]/y",
            "[-oProxyCommand=x]:y",
            "git@[-oX]:y",
            "ssh://[zz]/y",
            "ssh://[]/y",
        ] {
            assert_matches!(clone(bad), Err(NameError::CloneUrl { .. }), "{bad:?}");
        }
        assert!(clone("ssh://git@[::1]:22/y").is_ok());
    }
}
