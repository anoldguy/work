//! [`StoredTime`]: a UTC instant with an RFC 3339 wire format (pure, except
//! [`StoredTime::now`]).

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::NameError;

/// Nanoseconds in one second, as the `i128` span arithmetic uses.
const NANOS_PER_SEC: i128 = 1_000_000_000;

/// A UTC instant, stored in the cache as an RFC 3339 string.
///
/// The process reads the clock once, at `App` construction, and passes that
/// value down; nothing else calls [`StoredTime::now`]. Serde goes through
/// [`Display`](fmt::Display)/[`FromStr`], which pins the wire format
/// independently of jiff's own serde support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StoredTime(jiff::Timestamp);

impl StoredTime {
    /// Reads the system clock. Only the edge (`App::load`) calls this.
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
    /// RFC 3339 in UTC, the cache's wire format.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // jiff's `Timestamp` displays as RFC 3339 in UTC with a `Z` suffix.
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

    use proptest::prelude::*;

    use super::*;

    /// Seconds for 0000-01-01T00:00:00Z through 9999-12-31T23:59:59Z, the
    /// years RFC 3339 writes with four digits.
    const MIN_SECS: i64 = -62_167_219_200;
    const MAX_SECS: i64 = 253_402_300_799;

    #[test]
    fn from_unix_seconds_formats_as_utc() {
        let t = StoredTime::from_unix_seconds(1_700_000_000).expect("in range");
        assert_eq!(t.to_string(), "2023-11-14T22:13:20Z");
    }

    #[test]
    fn from_unix_seconds_rejects_out_of_range() {
        assert_matches!(
            StoredTime::from_unix_seconds(i64::MAX),
            Err(NameError::Time { .. })
        );
    }

    #[test]
    fn parse_normalises_offset_to_utc() {
        let parsed: StoredTime = "2024-01-01T02:00:00+02:00".parse().expect("valid");
        let utc: StoredTime = "2024-01-01T00:00:00Z".parse().expect("valid");
        assert_eq!(parsed, utc);
        assert_eq!(parsed.to_string(), "2024-01-01T00:00:00Z");
    }

    #[test]
    fn parse_error_names_the_input_and_reason() {
        let err = "yesterday"
            .parse::<StoredTime>()
            .expect_err("not a timestamp");
        assert_matches!(&err, NameError::Time { given, reason }
            if given == "yesterday" && *reason == "not an RFC 3339 timestamp");
        assert_eq!(
            err.to_string(),
            "invalid timestamp \"yesterday\" (not an RFC 3339 timestamp)"
        );
    }

    #[test]
    fn parse_rejects_missing_offset() {
        assert_matches!(
            "2024-01-01T00:00:00".parse::<StoredTime>(),
            Err(NameError::Time { .. })
        );
    }

    #[test]
    fn parse_rejects_empty_input() {
        assert_matches!("".parse::<StoredTime>(), Err(NameError::Time { .. }));
    }

    #[test]
    fn parse_error_escapes_control_characters() {
        let err = "2024\n".parse::<StoredTime>().expect_err("not a timestamp");
        assert!(!err.to_string().contains('\n'));
    }

    #[test]
    fn serde_uses_rfc3339_string() {
        let t = StoredTime::from_unix_seconds(0).expect("in range");
        assert_eq!(
            serde_json::to_string(&t).expect("serializes"),
            "\"1970-01-01T00:00:00Z\""
        );
        let back: StoredTime =
            serde_json::from_str("\"1970-01-01T01:00:00+01:00\"").expect("deserializes");
        assert_eq!(back, t);
    }

    #[test]
    fn serde_rejects_non_rfc3339_string() {
        assert_matches!(serde_json::from_str::<StoredTime>("\"nope\""), Err(_));
    }

    #[test]
    fn age_at_is_exact_for_past_instant() {
        let then = StoredTime::from_unix_seconds(1_000).expect("in range");
        let now = StoredTime::from_unix_seconds(1_090).expect("in range");
        assert_eq!(then.age_at(now), Duration::from_secs(90));
    }

    #[test]
    fn age_at_saturates_to_zero_when_now_is_before_stored_time() {
        let stored = StoredTime::from_unix_seconds(2_000).expect("in range");
        let earlier = StoredTime::from_unix_seconds(1_000).expect("in range");
        assert_eq!(stored.age_at(earlier), Duration::ZERO);
    }

    #[test]
    fn age_at_is_zero_for_same_instant() {
        let t = StoredTime::from_unix_seconds(5).expect("in range");
        assert_eq!(t.age_at(t), Duration::ZERO);
    }

    #[test]
    fn age_at_keeps_subsecond_precision() {
        let stored: StoredTime = "2024-01-01T00:00:00.250Z".parse().expect("valid");
        let now: StoredTime = "2024-01-01T00:00:01.000Z".parse().expect("valid");
        assert_eq!(stored.age_at(now), Duration::from_millis(750));
    }

    proptest! {
        #[test]
        fn rfc3339_round_trips(
            secs in jiff::Timestamp::MIN.as_second()..=jiff::Timestamp::MAX.as_second(),
            nanos in 0i32..1_000_000_000,
        ) {
            // Clamp the extreme second so secs + nanos stays in range.
            let Ok(ts) = jiff::Timestamp::new(secs, nanos) else { return Ok(()); };
            let original = StoredTime(ts);
            let wire = original.to_string();
            prop_assert!(wire.ends_with('Z'), "not UTC: {wire}");
            let parsed: StoredTime = wire.parse().expect("own output parses");
            prop_assert_eq!(parsed, original);
            let json = serde_json::to_string(&original).expect("serializes");
            let back: StoredTime = serde_json::from_str(&json).expect("deserializes");
            prop_assert_eq!(back, original);
        }

        #[test]
        fn age_at_never_negative_and_matches_gap(
            a in MIN_SECS..=MAX_SECS,
            b in MIN_SECS..=MAX_SECS,
        ) {
            let stored = StoredTime::from_unix_seconds(a).expect("in range");
            let now = StoredTime::from_unix_seconds(b).expect("in range");
            let age = stored.age_at(now);
            if b > a {
                prop_assert_eq!(age, Duration::from_secs(b.abs_diff(a)));
            } else {
                prop_assert_eq!(age, Duration::ZERO);
            }
        }
    }
}
