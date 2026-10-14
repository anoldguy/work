//! [`CloneUrl`], [`HtmlUrl`] and [`ApiUrl`]: URLs that are safe to hand to
//! git, a browser, or the HTTP client (pure).

use std::fmt;

use serde::{Deserialize, Serialize};

use super::NameError;

/// The parts of an absolute `scheme://authority...` URL.
struct Origin<'a> {
    scheme: &'a str,
    /// Host as written (IPv6 keeps its brackets), without user info or port.
    host: &'a str,
    port: Option<&'a str>,
    has_user_info: bool,
    /// User info before the `@`, empty when absent.
    user: &'a str,
    /// Everything after the authority (path, query, fragment).
    rest: &'a str,
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
    let (authority, rest) = after.split_at(end);
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
        port: port.filter(|p| !p.is_empty()),
        has_user_info,
        user,
        rest,
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

fn effective_port<'a>(o: &Origin<'a>) -> Option<&'a str> {
    o.port.or(if o.scheme.eq_ignore_ascii_case("https") {
        Some("443")
    } else if o.scheme.eq_ignore_ascii_case("http") {
        Some("80")
    } else {
        None
    })
}

fn is_loopback(host: &str) -> bool {
    matches!(
        host.to_ascii_lowercase().as_str(),
        "127.0.0.1" | "[::1]" | "localhost"
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

/// A repository's web page, opened by `browse`.
///
/// Invariant: `https://` with a non-empty host. That the host equals the
/// repo's domain is checked where the API response is converted into a
/// `Repo` (`github::dto`), because only that code knows the domain.
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

/// A REST API base URL, e.g. `https://api.github.com` or
/// `https://ghe.corp.example/api/v3`.
///
/// Invariant: `https://`, or `http://` only for loopback hosts (`127.0.0.1`,
/// `::1`, `localhost`; used by tests). No query, fragment, or user info; no
/// trailing slash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ApiUrl(String);

impl ApiUrl {
    /// The URL as a string slice, without a trailing slash (tests).
    #[cfg(test)]
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// True when `url` has the same scheme, host and port as this base. The
    /// pagination guard: a `next` link failing this is never followed.
    #[must_use]
    pub fn same_origin(&self, url: &str) -> bool {
        let (Some(base), Some(other)) = (split_origin(&self.0), split_origin(url)) else {
            return false;
        };
        if other.has_user_info || has_unsafe_chars(url) {
            return false;
        }
        base.scheme.eq_ignore_ascii_case(other.scheme)
            && base.host.eq_ignore_ascii_case(other.host)
            && effective_port(&base) == effective_port(&other)
    }
}

/// Why `s` is not an allowed API base URL, if it is not one.
fn api_url_problem(s: &str) -> Option<&'static str> {
    if has_unsafe_chars(s) {
        return Some("whitespace and control characters are not allowed");
    }
    let Some(o) = split_origin(s) else {
        return Some("must be an http(s) URL");
    };
    if o.has_user_info {
        return Some("user info is not allowed");
    }
    if o.rest.contains(['?', '#']) {
        return Some("query and fragment are not allowed");
    }
    if o.host.is_empty() {
        return Some("missing host");
    }
    if !host_brackets_ok(o.host) {
        return Some("malformed host");
    }
    if o.scheme == "https" {
        None
    } else if o.scheme == "http" {
        (!is_loopback(o.host)).then_some("plain http is only allowed for loopback hosts")
    } else {
        Some("must be an https URL")
    }
}

impl TryFrom<String> for ApiUrl {
    type Error = NameError;

    /// Validates an API base, dropping trailing slashes.
    ///
    /// # Errors
    ///
    /// [`NameError::ApiUrl`] for plain http on a non-loopback host, other
    /// schemes, or a query, fragment, or user info.
    fn try_from(mut value: String) -> Result<Self, Self::Error> {
        match api_url_problem(&value) {
            None => {
                // The authority never ends in '/', so this cannot eat the host.
                value.truncate(value.trim_end_matches('/').len());
                Ok(Self(value))
            }
            Some(reason) => Err(NameError::ApiUrl {
                given: value,
                reason,
            }),
        }
    }
}

impl From<ApiUrl> for String {
    fn from(value: ApiUrl) -> Self {
        value.0
    }
}

impl fmt::Display for ApiUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use proptest::prelude::*;

    use super::{ApiUrl, CloneUrl, HtmlUrl};
    use crate::domain::NameError;

    fn clone(s: &str) -> Result<CloneUrl, NameError> {
        CloneUrl::try_from(s.to_owned())
    }
    fn html(s: &str) -> Result<HtmlUrl, NameError> {
        HtmlUrl::try_from(s.to_owned())
    }
    fn api(s: &str) -> Result<ApiUrl, NameError> {
        ApiUrl::try_from(s.to_owned())
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
    fn api_url_allows_https_and_loopback_http_only() {
        for ok in [
            "https://api.github.com",
            "https://ghe.corp.example/api/v3",
            "http://127.0.0.1:8080",
            "http://localhost:1234",
            "http://[::1]:9",
        ] {
            assert_eq!(api(ok).unwrap().as_str(), ok);
        }
        for bad in [
            "http://api.github.com",
            "http://127.0.0.1.evil.com",
            "http://localhost.evil.com",
            "ftp://api.github.com",
            "api.github.com",
            "https://",
            "https://u:p@api.github.com",
            "https://api.github.com?x=1",
            "https://api.github.com/v3#frag",
            "https://api.github.com:port",
            "https://api. github.com",
        ] {
            assert_matches!(api(bad), Err(NameError::ApiUrl { .. }), "{bad:?}");
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
            assert_matches!(api(bad), Err(NameError::ApiUrl { .. }), "{bad:?}");
            assert_matches!(clone(bad), Err(NameError::CloneUrl { .. }), "{bad:?}");
        }
        assert!(
            !api("https://api.github.com")
                .unwrap()
                .same_origin("https://evil.com\\@api.github.com/x")
        );
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
        assert_matches!(api("https://[-x]"), Err(NameError::ApiUrl { .. }));
    }

    #[test]
    fn schemes_must_be_lowercase() {
        assert_matches!(clone("SSH://host/x"), Err(NameError::CloneUrl { .. }));
        assert_matches!(clone("HTTPS://host/x"), Err(NameError::CloneUrl { .. }));
        assert_matches!(html("HTTPS://host/x"), Err(NameError::HtmlUrl { .. }));
        assert_matches!(api("HTTPS://host"), Err(NameError::ApiUrl { .. }));
    }

    #[test]
    fn api_url_drops_every_trailing_slash() {
        assert_eq!(api("https://h//").unwrap().as_str(), "https://h");
        assert_eq!(api("https://h/api///").unwrap().as_str(), "https://h/api");
    }

    #[test]
    fn api_url_drops_one_trailing_slash() {
        assert_eq!(
            api("https://api.github.com/").unwrap().as_str(),
            "https://api.github.com"
        );
        assert_eq!(
            api("https://h/api/v3/").unwrap().as_str(),
            "https://h/api/v3"
        );
    }

    #[test]
    fn api_same_origin_compares_scheme_host_and_port() {
        let base = api("https://api.github.com").unwrap();
        assert!(base.same_origin("https://api.github.com/orgs?page=2"));
        assert!(base.same_origin("https://API.github.com:443/orgs"));
        assert!(!base.same_origin("http://api.github.com/orgs"));
        assert!(!base.same_origin("https://api.github.com:8443/orgs"));
        assert!(!base.same_origin("https://evil.com/orgs"));
        assert!(!base.same_origin("https://api.github.com.evil.com/orgs"));
        assert!(!base.same_origin("https://api.github.com@evil.com/orgs"));
        assert!(!base.same_origin("/orgs?page=2"));
        assert!(!base.same_origin(""));
        let local = api("http://127.0.0.1:8080").unwrap();
        assert!(local.same_origin("http://127.0.0.1:8080/x"));
        assert!(!local.same_origin("http://127.0.0.1:8081/x"));
        assert!(!local.same_origin("http://127.0.0.1/x"));
    }

    #[test]
    fn urls_serialise_as_strings_and_validate_on_deserialise() {
        let url = clone("https://github.com/a/b.git").unwrap();
        let json = serde_json::to_string(&url).unwrap();
        assert_eq!(json, "\"https://github.com/a/b.git\"");
        assert_eq!(serde_json::from_str::<CloneUrl>(&json).unwrap(), url);
        assert!(serde_json::from_str::<CloneUrl>("\"ext::evil\"").is_err());
        assert!(serde_json::from_str::<HtmlUrl>("\"http://x/\"").is_err());
        assert!(serde_json::from_str::<ApiUrl>("\"http://x.com\"").is_err());
    }

    proptest! {
        #[test]
        fn https_urls_round_trip(host in "[a-z0-9]{1,10}(\\.[a-z0-9]{1,5}){0,2}", path in "(/[A-Za-z0-9._-]{1,8}){0,3}") {
            let url = format!("https://{host}{path}");
            let c = clone(&url).unwrap();
            prop_assert_eq!(String::from(c), url.clone());
            let h = html(&url).unwrap();
            prop_assert_eq!(h.to_string(), url.clone());
            let a = api(&url).unwrap();
            prop_assert!(a.same_origin(&url));
            let json = serde_json::to_string(&a).unwrap();
            prop_assert_eq!(serde_json::from_str::<ApiUrl>(&json).unwrap(), a);
        }

        #[test]
        fn clone_url_never_accepts_leading_dash_or_ext(tail in "[ -~]{0,20}") {
            let dashed = format!("-{tail}");
            let ext = format!("ext::{tail}");
            prop_assert!(clone(&dashed).is_err());
            prop_assert!(clone(&ext).is_err());
        }

        #[test]
        fn clone_url_rejects_dash_user_in_ssh_forms(user in "-[A-Za-z0-9=]{0,10}") {
            let ssh = format!("ssh://{user}@host/x");
            let scp = format!("{user}@host:x");
            prop_assert!(clone(&ssh).is_err());
            prop_assert!(clone(&scp).is_err());
        }

        #[test]
        fn api_url_with_many_trailing_slashes_normalises(path in "(/[a-z]{1,4}){0,3}/{0,5}") {
            let a = api(&format!("https://h{path}")).unwrap();
            prop_assert!(!a.as_str().ends_with('/'));
            prop_assert_eq!(api(a.as_str()).unwrap(), a);
        }

        #[test]
        fn api_url_is_idempotent_and_slashless(s in "[ -~]{0,30}") {
            if let Ok(a) = api(&s) {
                prop_assert!(!a.as_str().ends_with('/'));
                prop_assert_eq!(api(a.as_str()).unwrap(), a);
            }
        }

        #[test]
        fn plain_http_is_rejected_for_non_loopback_hosts(host in "[a-z]{1,10}\\.[a-z]{2,5}") {
            assert_matches!(api(&format!("http://{host}")), Err(NameError::ApiUrl { .. }));
        }
    }
}
