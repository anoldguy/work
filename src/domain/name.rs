//! Host and account names: [`Domain`], [`Org`], [`RepoName`] (pure).
//!
//! These validators are part of the contract (not a later chunk) because
//! nearly every module needs real values in its tests.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::NameError;

/// Longest hostname accepted (RFC 1035).
const MAX_DOMAIN_LEN: usize = 253;
/// Longest hostname label accepted (RFC 1035).
const MAX_LABEL_LEN: usize = 63;
/// Longest login, org or repository name accepted (GitHub allows 100 for
/// repository names; logins are shorter).
const MAX_NAME_LEN: usize = 100;

/// A GitHub host, such as `github.com` or `ghe.corp.example`.
///
/// Invariant: a canonical lowercase hostname. Labels are 1–63 characters of
/// `[a-z0-9-]`, not starting or ending with `-`, separated by single dots;
/// no scheme, port, path, or user info.
///
/// [`TryFrom<String>`] (used by serde for config and cache files) rejects
/// uppercase, so `[domains."GitHub.com"]` is an error rather than a silent
/// alias. [`FromStr`] (used for command-line arguments) lowercases first.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Domain(String);

impl Domain {
    /// The hostname as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Why `s` is not a canonical lowercase hostname, if it is not one.
fn domain_problem(s: &str) -> Option<&'static str> {
    if s.is_empty() {
        return Some("empty");
    }
    if s.len() > MAX_DOMAIN_LEN {
        return Some("longer than 253 characters");
    }
    if s.chars().any(|c| c.is_ascii_uppercase()) {
        return Some("must be lowercase");
    }
    s.split('.').find_map(|label| {
        if label.is_empty() {
            Some("empty label")
        } else if label.len() > MAX_LABEL_LEN {
            Some("label longer than 63 characters")
        } else if !label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            Some("only letters, digits, '-' and '.' are allowed")
        } else if label.starts_with('-') || label.ends_with('-') {
            Some("a label cannot start or end with '-'")
        } else {
            None
        }
    })
}

impl TryFrom<String> for Domain {
    type Error = NameError;

    /// Validates a canonical (lowercase) hostname.
    ///
    /// # Errors
    ///
    /// [`NameError::Domain`] when the input is not a bare lowercase hostname.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match domain_problem(&value) {
            None => Ok(Self(value)),
            Some(reason) => Err(NameError::Domain {
                given: value,
                reason,
            }),
        }
    }
}

impl FromStr for Domain {
    type Err = NameError;

    /// Lowercases, then validates (for user-typed hostnames).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s.to_ascii_lowercase())
    }
}

impl From<Domain> for String {
    fn from(value: Domain) -> Self {
        value.0
    }
}

impl fmt::Display for Domain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why `s` is not a valid login, org or repository name, if it is not one.
fn name_problem(s: &str) -> Option<&'static str> {
    if s.is_empty() {
        Some("empty")
    } else if s.len() > MAX_NAME_LEN {
        Some("longer than 100 characters")
    } else if s == "." || s == ".." {
        Some("'.' and '..' are not names")
    } else if s.starts_with('-') {
        Some("cannot start with '-'")
    } else if !s
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        Some("only letters, digits, '.', '_' and '-' are allowed")
    } else {
        None
    }
}

/// A GitHub login or organisation name.
///
/// Invariant: 1–100 characters of `[A-Za-z0-9._-]`, not `.` or `..`, no
/// leading `-`. Case is preserved as the API returns it; filters compare with
/// [`Org::eq_ignore_case`], while frecency keys compare exactly.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Org(String);

impl Org {
    /// The name as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// ASCII case-insensitive comparison, used by include/exclude filters,
    /// favourites, and extra-org de-duplication.
    #[must_use]
    pub fn eq_ignore_case(&self, other: &Self) -> bool {
        self.0.eq_ignore_ascii_case(&other.0)
    }
}

impl TryFrom<String> for Org {
    type Error = NameError;

    /// Validates a login or organisation name.
    ///
    /// # Errors
    ///
    /// [`NameError::Org`] when the input breaks the charset rules.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match name_problem(&value) {
            None => Ok(Self(value)),
            Some(reason) => Err(NameError::Org {
                given: value,
                reason,
            }),
        }
    }
}

impl FromStr for Org {
    type Err = NameError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s.to_owned())
    }
}

impl From<Org> for String {
    fn from(value: Org) -> Self {
        value.0
    }
}

impl fmt::Display for Org {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A repository name, the last segment of a slug.
///
/// Invariant: same rules as [`Org`]; case preserved.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RepoName(String);

impl RepoName {
    /// The name as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for RepoName {
    type Error = NameError;

    /// Validates a repository name.
    ///
    /// # Errors
    ///
    /// [`NameError::RepoName`] when the input breaks the charset rules.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match name_problem(&value) {
            None => Ok(Self(value)),
            Some(reason) => Err(NameError::RepoName {
                given: value,
                reason,
            }),
        }
    }
}

impl FromStr for RepoName {
    type Err = NameError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s.to_owned())
    }
}

impl From<RepoName> for String {
    fn from(value: RepoName) -> Self {
        value.0
    }
}

impl fmt::Display for RepoName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::{Domain, Org, RepoName};
    use crate::domain::NameError;

    #[test]
    fn domain_accepts_hostnames() {
        for ok in ["github.com", "ghe.corp-1.example", "localhost", "a.b.c"] {
            assert_eq!(
                Domain::try_from(ok.to_owned()).map(String::from).as_deref(),
                Ok(ok)
            );
        }
    }

    #[test]
    fn domain_rejects_schemes_ports_paths_and_bad_labels() {
        for bad in [
            "",
            "https://github.com",
            "github.com:443",
            "github.com/acme",
            "github..com",
            ".github.com",
            "-github.com",
            "github-.com",
            "git hub.com",
            "github.com\n",
        ] {
            assert_matches!(
                Domain::try_from(bad.to_owned()),
                Err(NameError::Domain { .. }),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn domain_files_must_be_lowercase_but_arguments_are_lowercased() {
        assert_matches!(
            Domain::try_from("GitHub.com".to_owned()),
            Err(NameError::Domain { .. })
        );
        assert_eq!(
            "GitHub.com".parse::<Domain>().map(String::from).as_deref(),
            Ok("github.com")
        );
    }

    #[test]
    fn names_keep_case_and_reject_unsafe_forms() {
        assert_eq!(
            "My.Org_1-x".parse::<Org>().map(String::from).as_deref(),
            Ok("My.Org_1-x")
        );
        for bad in ["", ".", "..", "-rf", "a/b", "a b", "ä"] {
            assert_matches!(bad.parse::<Org>(), Err(NameError::Org { .. }), "{bad:?}");
            assert_matches!(
                bad.parse::<RepoName>(),
                Err(NameError::RepoName { .. }),
                "{bad:?}"
            );
        }
        assert_matches!(
            "x".repeat(101).parse::<RepoName>(),
            Err(NameError::RepoName { .. })
        );
    }

    #[test]
    fn org_comparison_for_filters_ignores_case() {
        let a: Org = "Acme".parse().expect("valid");
        let b: Org = "acme".parse().expect("valid");
        assert!(a.eq_ignore_case(&b));
        assert_ne!(a, b);
    }
}
