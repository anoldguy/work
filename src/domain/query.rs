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
    /// characters.
    pub fn new(raw: &str) -> Result<Self, NameError> {
        let _ = raw;
        unimplemented!("W1.1")
    }

    /// The key as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
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
