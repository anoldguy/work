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
    /// [`NameError::Slug`] when there are not exactly three segments or a
    /// segment is invalid; the reason names the failing segment's problem.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let parsed = {
            let segments: Vec<&str> = value.split('/').collect();
            let [domain, org, name] = segments[..] else {
                return Err(NameError::Slug {
                    given: value,
                    reason: "expected {domain}/{org}/{repo}",
                });
            };
            Domain::try_from(domain.to_owned())
                .and_then(|d| Ok((d, Org::try_from(org.to_owned())?)))
                .and_then(|(d, o)| Ok((d, o, RepoName::try_from(name.to_owned())?)))
        };
        match parsed {
            Ok((domain, org, name)) => Ok(Self { domain, org, name }),
            Err(
                NameError::Domain { reason, .. }
                | NameError::Org { reason, .. }
                | NameError::RepoName { reason, .. }
                | NameError::Slug { reason, .. },
            ) => Err(NameError::Slug {
                given: value,
                reason,
            }),
            Err(other) => Err(other),
        }
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

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use proptest::prelude::*;

    use super::Slug;
    use crate::domain::NameError;

    #[test]
    fn slug_parses_and_displays_round_trip() {
        let slug: Slug = "github.com/Acme/widget.rs".parse().expect("valid");
        assert_eq!(slug.domain().as_str(), "github.com");
        assert_eq!(slug.org().as_str(), "Acme");
        assert_eq!(slug.name().as_str(), "widget.rs");
        assert_eq!(slug.to_string(), "github.com/Acme/widget.rs");
    }

    #[test]
    fn slug_rejects_wrong_segment_counts() {
        for bad in ["", "github.com", "github.com/acme", "github.com/a/b/c"] {
            assert_matches!(bad.parse::<Slug>(), Err(NameError::Slug { .. }), "{bad:?}");
        }
    }

    #[test]
    fn slug_rejects_unsafe_segments() {
        for bad in [
            "github.com/../x",
            "github.com/acme/..",
            "github.com/-rf/x",
            "github.com/acme/-x",
            "github.com:443/acme/x",
            "GitHub.com/acme/x",
            "github.com/ac me/x",
        ] {
            assert!(bad.parse::<Slug>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn slug_rejects_git_metadata_segments_in_any_case() {
        for bad in [
            "github.com/acme/.git",
            "github.com/acme/.GIT",
            "github.com/.git/x",
            "github.com/acme/.git.",
        ] {
            assert_matches!(bad.parse::<Slug>(), Err(NameError::Slug { .. }), "{bad:?}");
        }
    }

    #[test]
    fn slug_segment_errors_keep_the_whole_slug() {
        let err = "github.com/../x".parse::<Slug>().unwrap_err();
        assert_matches!(&err, NameError::Slug { given, .. } if given == "github.com/../x");
    }

    #[test]
    fn slug_serialises_as_a_plain_string() {
        let slug: Slug = "github.com/acme/x".parse().expect("valid");
        let json = serde_json::to_string(&slug).expect("serialise");
        assert_eq!(json, "\"github.com/acme/x\"");
        assert_eq!(serde_json::from_str::<Slug>(&json).expect("parse"), slug);
        assert!(serde_json::from_str::<Slug>("\"github.com/acme\"").is_err());
    }

    proptest! {
        #[test]
        fn slug_round_trips_through_string_and_json(
            d in "[a-z0-9]([a-z0-9-]{0,10}[a-z0-9])?(\\.[a-z0-9]{1,5}){0,2}",
            o in "[A-Za-z0-9_][A-Za-z0-9._-]{0,20}",
            r in "[A-Za-z0-9_][A-Za-z0-9._-]{0,20}",
        ) {
            let text = format!("{d}/{o}/{r}");
            let slug: Slug = text.parse().unwrap();
            prop_assert_eq!(slug.to_string(), text);
            let json = serde_json::to_string(&slug).unwrap();
            prop_assert_eq!(serde_json::from_str::<Slug>(&json).unwrap(), slug);
        }

        #[test]
        fn slug_parse_never_accepts_what_display_would_not_reproduce(s in ".{0,40}") {
            if let Ok(slug) = s.parse::<Slug>() {
                prop_assert_eq!(slug.to_string(), s);
            }
        }
    }
}
