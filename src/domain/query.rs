//! [`QueryKey`]: a normalised repo query (pure).

use std::fmt;

use serde::{Deserialize, Serialize};

use super::NameError;

/// A normalised query string: trimmed and lowercased.
///
/// Invariant: non-empty, no control characters, already normalised, so
/// normalising again is a no-op. That makes it safe to show in a one-line
/// error with `{}`. Serialised as the string; it is the outer key of the
/// cache's `query_frecency` map.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct QueryKey(String);

impl QueryKey {
    /// Trims and lowercases `raw`.
    ///
    /// # Errors
    ///
    /// [`NameError::Query`] when the result is empty or contains control
    /// characters (including invisible bidi and separator characters).
    pub fn new(raw: &str) -> Result<Self, NameError> {
        let key = raw.trim().to_lowercase();
        let reason = if key.is_empty() {
            "empty"
        } else if key
            .chars()
            .any(|c| c.is_control() || is_invisible_format(c))
        {
            "control characters are not allowed"
        } else {
            return Ok(Self(key));
        };
        Err(NameError::Query {
            given: raw.to_owned(),
            reason,
        })
    }

    /// The key as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Bidi controls, zero-width characters and line/paragraph separators:
/// invisible characters that could disguise a key in a one-line message.
const fn is_invisible_format(c: char) -> bool {
    matches!(
        c,
        '\u{AD}'
            | '\u{200B}'..='\u{200F}'
            | '\u{2028}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
    )
}

impl TryFrom<String> for QueryKey {
    type Error = NameError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(&value)
    }
}

impl From<QueryKey> for String {
    fn from(value: QueryKey) -> Self {
        value.0
    }
}

impl fmt::Display for QueryKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use proptest::prelude::*;

    use super::QueryKey;
    use crate::domain::NameError;

    #[test]
    fn query_is_trimmed_and_lowercased() {
        assert_eq!(
            QueryKey::new("  Acme/Widget \t").unwrap().as_str(),
            "acme/widget"
        );
    }

    #[test]
    fn query_rejects_empty_and_control_characters() {
        for bad in [
            "",
            "   ",
            "a\u{0}b",
            "a\nb",
            "a\u{1b}[0m",
            "\u{7f}x",
            "a\u{202E}b",
            "a\u{2028}b",
            "a\u{200B}b",
            "a\u{2029}b",
        ] {
            assert_matches!(QueryKey::new(bad), Err(NameError::Query { .. }), "{bad:?}");
        }
    }

    #[test]
    fn query_deserialisation_normalises() {
        let key: QueryKey = serde_json::from_str("\" Foo \"").unwrap();
        assert_eq!(key.as_str(), "foo");
        assert!(serde_json::from_str::<QueryKey>("\"\"").is_err());
    }

    proptest! {
        #[test]
        fn query_normalisation_is_idempotent(s in ".{0,30}") {
            if let Ok(key) = QueryKey::new(&s) {
                let again = QueryKey::new(key.as_str()).unwrap();
                prop_assert_eq!(again, key);
            }
        }

        #[test]
        fn query_never_contains_control_characters(s in ".{0,30}") {
            if let Ok(key) = QueryKey::new(&s) {
                prop_assert!(!key.as_str().chars().any(char::is_control));
            }
        }
    }
}
