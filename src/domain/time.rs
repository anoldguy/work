//! [`StoredTime`]: a UTC instant with an RFC 3339 wire format (pure, except
//! [`StoredTime::now`]).

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::NameError;

/// Nanoseconds in one second, as the `i128` span arithmetic uses.
const NANOS_PER_SEC: i128 = 1_000_000_000;

/// A UTC instant, serialised as an RFC 3339 string.
///
/// Serde goes through [`Display`](fmt::Display)/[`FromStr`], so the wire
/// format does not depend on jiff's own serde support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StoredTime(jiff::Timestamp);

impl StoredTime {
    /// Reads the system clock.
    #[must_use]
    pub fn now() -> Self {
        Self(jiff::Timestamp::now())
    }

    /// Builds a time from whole seconds since the Unix epoch.
    ///
    /// # Errors
    ///
    /// [`NameError::Time`] when `seconds` is outside jiff's supported range.
    #[cfg(test)]
    pub fn from_unix_seconds(seconds: i64) -> Result<Self, NameError> {
        jiff::Timestamp::from_second(seconds)
            .map(Self)
            .map_err(|_| NameError::Time {
                given: seconds.to_string(),
                reason: "outside the supported range",
            })
    }

    /// How long before `now` this instant was; zero if it is in the future.
    #[must_use]
    pub fn age_at(self, now: Self) -> Duration {
        let nanos = now.0.as_nanosecond() - self.0.as_nanosecond();
        if nanos <= 0 {
            return Duration::ZERO;
        }
        let secs = u64::try_from(nanos / NANOS_PER_SEC).unwrap_or(u64::MAX);
        let subsec = u32::try_from(nanos % NANOS_PER_SEC).unwrap_or(0);
        Duration::new(secs, subsec)
    }
}

impl fmt::Display for StoredTime {
    /// RFC 3339 in UTC with a `Z` suffix.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl FromStr for StoredTime {
    type Err = NameError;

    /// Parses jiff's Temporal-flavoured ISO 8601 timestamps (a superset of
    /// RFC 3339: it also accepts a space separator, lowercase `t`/`z`, basic
    /// format and bracketed zone names). Any offset is normalised to UTC.
    /// [`Display`](fmt::Display) always writes strict RFC 3339.
    ///
    /// # Errors
    ///
    /// [`NameError::Time`] when `s` is not an ISO 8601 / RFC 3339 timestamp.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<jiff::Timestamp>()
            .map(Self)
            .map_err(|_| NameError::Time {
                given: s.to_owned(),
                reason: "not an RFC 3339 timestamp",
            })
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

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;

    #[test]
    fn parse_normalises_offset_to_utc() {
        let parsed: StoredTime = "2024-01-01T02:00:00+02:00".parse().expect("valid");
        let utc: StoredTime = "2024-01-01T00:00:00Z".parse().expect("valid");
        assert_eq!(parsed, utc);
        assert_eq!(parsed.to_string(), "2024-01-01T00:00:00Z");
    }

    #[test]
    fn parse_rejects_missing_offset() {
        assert_matches!(
            "2024-01-01T00:00:00".parse::<StoredTime>(),
            Err(NameError::Time { .. })
        );
    }

    #[test]
    fn age_at_saturates_to_zero_when_now_is_before_stored_time() {
        let stored = StoredTime::from_unix_seconds(2_000).expect("in range");
        let earlier = StoredTime::from_unix_seconds(1_000).expect("in range");
        assert_eq!(stored.age_at(earlier), Duration::ZERO);
    }
}
