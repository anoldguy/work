//! [`CloneUrl`], [`HtmlUrl`] and [`ApiUrl`]: URLs that are safe to hand to
//! git, a browser, or the HTTP client (pure).

use std::fmt;

use serde::{Deserialize, Serialize};

use super::{Domain, NameError};

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

impl TryFrom<String> for CloneUrl {
    type Error = NameError;

    /// Validates a clone URL.
    ///
    /// # Errors
    ///
    /// [`NameError::CloneUrl`] for any other scheme, a leading `-`, or `ext::`.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let _ = value;
        unimplemented!("W1.1")
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

    /// True when the URL's host (ignoring ASCII case and any port) is `d`.
    /// The DTO check that a repo's web page is on its own domain.
    #[must_use]
    pub fn host_matches(&self, d: &Domain) -> bool {
        let _ = (self, d);
        unimplemented!("W1.1")
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
        let _ = value;
        unimplemented!("W1.1")
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
    /// The URL as a string slice, without a trailing slash.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// True when `url` has the same scheme, host and port as this base. The
    /// pagination guard: a `next` link failing this is never followed.
    #[must_use]
    pub fn same_origin(&self, url: &str) -> bool {
        let _ = (self, url);
        unimplemented!("W1.1")
    }
}

impl TryFrom<String> for ApiUrl {
    type Error = NameError;

    /// Validates an API base, dropping one trailing slash.
    ///
    /// # Errors
    ///
    /// [`NameError::ApiUrl`] for plain http on a non-loopback host, other
    /// schemes, or a query, fragment, or user info.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let _ = value;
        unimplemented!("W1.1")
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
