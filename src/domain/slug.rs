//! [`Slug`]: the `{domain}/{org}/{repo}` identity of a repository (pure).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize, Serializer};

use super::{Domain, NameError, Org, RepoName};

/// A repository's full identity, `{domain}/{org}/{repo}`.
///
/// Serialised as that string (written with `collect_str`, so saving the
/// cache does not allocate a `String` per frecency key), which is also its
/// key in the frecency maps.
/// Ordering compares domain, then org, then name; this is the tie-break
/// order for search results.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(try_from = "String")]
pub struct Slug {
    domain: Domain,
    org: Org,
    name: RepoName,
}

impl Slug {
    /// Assembles a slug from parts that are already valid.
    #[must_use]
    pub const fn new(domain: Domain, org: Org, name: RepoName) -> Self {
        Self { domain, org, name }
    }

    /// The host part.
    #[must_use]
    pub const fn domain(&self) -> &Domain {
        &self.domain
    }

    /// The owner part (user or organisation).
    #[must_use]
    pub const fn org(&self) -> &Org {
        &self.org
    }

    /// The repository name part.
    #[must_use]
    pub const fn name(&self) -> &RepoName {
        &self.name
    }
}

impl TryFrom<String> for Slug {
    type Error = NameError;

    /// Parses `{domain}/{org}/{repo}`, validating each part.
    ///
    /// # Errors
    ///
    /// [`NameError::Slug`] when there are not exactly three segments, or the
    /// part's own error when a segment is invalid.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let _ = value;
        unimplemented!("W1.1")
    }
}

impl FromStr for Slug {
    type Err = NameError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s.to_owned())
    }
}

impl From<Slug> for String {
    fn from(value: Slug) -> Self {
        value.to_string()
    }
}

impl Serialize for Slug {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl fmt::Display for Slug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}/{}", self.domain, self.org, self.name)
    }
}
