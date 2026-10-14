//! [`StoredTime`]: a UTC instant with an RFC 3339 wire format (pure, except
//! [`StoredTime::now`]).

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::NameError;

/// A UTC instant, stored in the cache as an RFC 3339 string.
///
/// The process reads the clock once, at `App` construction, and passes that
/// value down; nothing else calls [`StoredTime::now`]. Serde goes through
/// [`Display`](fmt::Display)/[`FromStr`], which pins the wire format
/// independently of jiff's own serde support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StoredTime(jiff::Timestamp);

impl StoredTime {
    /// Reads the system clock. Only the edge (`App::new`) calls this.
    #[must_use]
    pub fn now() -> Self {
        Self(jiff::Timestamp::now())
    }

    /// Builds a time from whole seconds since the Unix epoch (for tests and
    /// fixtures).
    ///
    /// # Errors
    ///
    /// [`NameError::Time`] when `seconds` is outside jiff's supported range.
    pub fn from_unix_seconds(seconds: i64) -> Result<Self, NameError> {
        let _ = seconds;
        unimplemented!("W1.2")
    }

    /// How long before `now` this instant was; zero if it is in the future.
    #[must_use]
    pub fn age_at(self, now: Self) -> Duration {
        let _ = (self, now);
        unimplemented!("W1.2")
    }
}

impl fmt::Display for StoredTime {
    /// RFC 3339 in UTC, the cache's wire format.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = (self, f);
        unimplemented!("W1.2")
    }
}

impl FromStr for StoredTime {
    type Err = NameError;

    /// Parses RFC 3339 (any offset; normalised to UTC).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let _ = s;
        unimplemented!("W1.2")
    }
}

impl Serialize for StoredTime {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for StoredTime {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}
