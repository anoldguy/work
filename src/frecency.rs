//! `frecency.json`: repo and query frecency, stored apart from the
//! repository cache.
//!
//! Only navigation commands write it, always as a read-modify-write under
//! `frecency.json.lock` ([`record`]). A cache refresh never reads or writes
//! it. Scoring lives in [`crate::core::frecency`] and
//! [`crate::core::query_frecency`].

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::core::frecency::{Frecency, FrecencyData};
use crate::core::query_frecency::QueryFrecency;
use crate::domain::{Slug, StoredTime};
use crate::fsutil::atomic_write;
use crate::text::one_line;

/// File name inside the cache directory.
pub const FRECENCY_FILE: &str = "frecency.json";

/// The only version this build reads or writes.
pub const FRECENCY_VERSION: u32 = 1;

/// Entries whose latest access is older than this are dropped on write.
pub const RETENTION: Duration = Duration::from_hours(365 * 24);

/// Repo and query frecency: the contents of `frecency.json` minus its
/// version. A missing map reads as empty.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct History {
    /// Repo frecency, keyed by slug.
    #[serde(default)]
    pub frecency: Frecency,
    /// Query-to-repo frecency.
    #[serde(default)]
    pub query_frecency: QueryFrecency,
}

impl History {
    /// Drops every entry whose latest access is more than [`RETENTION`]
    /// before `now`, and every query left with no entries.
    pub fn prune(&mut self, now: StoredTime) {
        let fresh = |_: &Slug, d: &FrecencyData| d.last_access.age_at(now) <= RETENTION;
        self.frecency.retain(fresh);
        self.query_frecency.retain(fresh);
    }
}

/// The file as written.
#[derive(Serialize)]
struct FileOut<'a> {
    version: u32,
    #[serde(flatten)]
    history: &'a History,
}

/// Just the version, read first so another version is not mistaken for a
/// corrupt file. A file without one is corrupt.
#[derive(Deserialize)]
struct Peek {
    version: u32,
}

/// Something wrong with `frecency.json`, reported as one warning. Paths
/// render with `{:?}` to stay on one line.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Problem {
    /// The file has a version this build does not know. It is left alone
    /// and frecency is read-only.
    #[error("{path:?} has unknown version {version}; frecency is read-only for this run")]
    UnknownVersion {
        /// The file.
        path: PathBuf,
        /// The version it declares.
        version: u32,
    },
    /// The file could not be read or parsed. It was renamed to
    /// `frecency.json.corrupt` (unless that failed) and history starts
    /// empty.
    #[error(
        "{path:?} is unreadable ({reason}); moved it to {FRECENCY_FILE}.corrupt and started a new history"
    )]
    Corrupt {
        /// The file.
        path: PathBuf,
        /// Why (one line).
        reason: String,
    },
}

/// Saving `frecency.json` failed. Paths render with `{:?}` to stay on one
/// line.
#[derive(Debug, thiserror::Error)]
pub enum FrecencyError {
    /// Writing the file failed.
    #[error("cannot save frecency {path:?}: {source}")]
    Write {
        /// The file.
        path: PathBuf,
        /// The I/O error.
        source: io::Error,
    },
    /// Serialising failed.
    #[error("cannot encode frecency: {source}")]
    Encode {
        /// The `serde_json` error (single-line).
        source: serde_json::Error,
    },
}

/// What reading the file found.
enum Read {
    Missing,
    Loaded(History),
    UnknownVersion(u32),
    Corrupt(String),
}

/// `{dir}/frecency.json`.
#[must_use]
pub fn frecency_file(dir: &Path) -> PathBuf {
    dir.join(FRECENCY_FILE)
}

fn read(path: &Path) -> Read {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Read::Missing,
        Err(e) => return Read::Corrupt(one_line(&format!("cannot read file: {e}"))),
    };
    let parsed = serde_json::from_slice::<Peek>(&bytes).and_then(|Peek { version }| {
        if version == FRECENCY_VERSION {
            serde_json::from_slice::<History>(&bytes).map(Read::Loaded)
        } else {
            Ok(Read::UnknownVersion(version))
        }
    });
    parsed.unwrap_or_else(|e| Read::Corrupt(one_line(&e.to_string())))
}

/// Renames a corrupt file to `frecency.json.corrupt`, replacing an earlier
/// one. Best effort: if the rename fails, the next write replaces the file.
fn quarantine(path: &Path) {
    let mut to = path.as_os_str().to_owned();
    to.push(".corrupt");
    let to = PathBuf::from(to);
    if fs::symlink_metadata(&to).is_ok_and(|m| m.is_dir()) {
        let _ = fs::remove_dir_all(&to);
    }
    let _ = fs::rename(path, &to);
}

/// The history in `dir`, and the problem to warn about, if any. A missing
/// file is an empty history; an unknown version is an empty history and
/// [`Problem::UnknownVersion`]; an unreadable file is renamed to
/// `frecency.json.corrupt` and is an empty history and [`Problem::Corrupt`].
#[must_use]
pub fn load(dir: &Path) -> (History, Option<Problem>) {
    let path = frecency_file(dir);
    match read(&path) {
        Read::Missing => (History::default(), None),
        Read::Loaded(history) => (history, None),
        Read::UnknownVersion(version) => (
            History::default(),
            Some(Problem::UnknownVersion { path, version }),
        ),
        Read::Corrupt(reason) => {
            quarantine(&path);
            (History::default(), Some(Problem::Corrupt { path, reason }))
        }
    }
}

/// Takes the exclusive advisory lock on `{dir}/frecency.json.lock`, released
/// when the file is dropped. Best effort: `None` when the lock file cannot be
/// created or the filesystem cannot lock; the write itself is still atomic.
fn lock(dir: &Path) -> Option<fs::File> {
    fs::create_dir_all(dir).ok()?;
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(format!("{FRECENCY_FILE}.lock")))
        .ok()?;
    file.lock().ok()?;
    Some(file)
}

/// Records into `{dir}/frecency.json`: under the lock, reloads the file,
/// applies `f`, prunes entries older than [`RETENTION`] before `now`, and
/// writes the result atomically, so concurrent `work` processes never lose
/// each other's accesses.
///
/// A file of unknown version is never overwritten: nothing is written and
/// the result is `Ok(Some(Problem::UnknownVersion))`. A file that became
/// unreadable is quarantined first and the result carries
/// [`Problem::Corrupt`].
///
/// # Errors
///
/// [`FrecencyError`] when encoding or writing fails.
pub fn record(
    dir: &Path,
    now: StoredTime,
    f: impl FnOnce(&mut History),
) -> Result<Option<Problem>, FrecencyError> {
    let _lock = lock(dir);
    let path = frecency_file(dir);
    let (mut history, problem) = match read(&path) {
        Read::Missing => (History::default(), None),
        Read::Loaded(history) => (history, None),
        Read::UnknownVersion(version) => {
            return Ok(Some(Problem::UnknownVersion { path, version }));
        }
        Read::Corrupt(reason) => {
            quarantine(&path);
            (
                History::default(),
                Some(Problem::Corrupt {
                    path: path.clone(),
                    reason,
                }),
            )
        }
    };
    f(&mut history);
    history.prune(now);
    let file = FileOut {
        version: FRECENCY_VERSION,
        history: &history,
    };
    let mut json =
        serde_json::to_vec_pretty(&file).map_err(|source| FrecencyError::Encode { source })?;
    json.push(b'\n');
    atomic_write(&path, &json).map_err(|source| FrecencyError::Write { path, source })?;
    Ok(problem)
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::domain::QueryKey;

    const DAY: i64 = 86_400;
    const NOW: i64 = 1_000 * DAY;

    fn t(secs: i64) -> StoredTime {
        StoredTime::from_unix_seconds(secs).unwrap()
    }

    fn slug(name: &str) -> Slug {
        Slug::try_from(format!("github.com/acme/{name}")).unwrap()
    }

    fn count(h: &History, name: &str) -> Option<u32> {
        h.frecency.get(&slug(name)).map(|d| d.access_count)
    }

    #[test]
    fn records_round_trip_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("state");
        let key = QueryKey::new("ap").unwrap();
        record(&sub, t(NOW), |h| {
            h.frecency.record(&slug("api"), t(NOW));
            h.query_frecency.record(&key, &slug("api"), t(NOW));
        })
        .unwrap();
        let (h, problem) = load(&sub);
        assert_eq!(problem, None);
        assert_eq!(count(&h, "api"), Some(1));
        assert!(h.query_frecency.get(&key).is_some());
        let json: serde_json::Value =
            serde_json::from_slice(&fs::read(frecency_file(&sub)).unwrap()).unwrap();
        assert_eq!(json["version"], 1);
    }

    #[test]
    fn a_write_drops_entries_older_than_a_year() {
        let dir = tempfile::tempdir().unwrap();
        let key = QueryKey::new("x").unwrap();
        record(dir.path(), t(NOW), |h| {
            h.frecency.record(&slug("old"), t(NOW - 366 * DAY));
            h.frecency.record(&slug("kept"), t(NOW - 365 * DAY));
            h.query_frecency
                .record(&key, &slug("old"), t(NOW - 366 * DAY));
        })
        .unwrap();
        let (h, _) = load(dir.path());
        assert_eq!(count(&h, "old"), None);
        assert_eq!(count(&h, "kept"), Some(1));
        assert!(h.query_frecency.get(&key).is_none());
    }

    #[test]
    fn a_corrupt_file_is_renamed_warned_about_and_replaced_by_an_empty_history() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(frecency_file(dir.path()), "{not json").unwrap();
        let (h, problem) = load(dir.path());
        assert_eq!(h, History::default());
        assert_matches!(problem, Some(p @ Problem::Corrupt { .. })
            if p.to_string().contains("frecency.json") && !p.to_string().contains('\n'));
        assert_eq!(
            fs::read_to_string(dir.path().join("frecency.json.corrupt")).unwrap(),
            "{not json"
        );
        assert!(!frecency_file(dir.path()).exists());
    }

    #[test]
    fn an_unknown_version_is_read_only_and_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let body = r#"{"version":2,"frecency":{"anything":"goes"}}"#;
        fs::write(frecency_file(dir.path()), body).unwrap();
        let (h, problem) = load(dir.path());
        assert_eq!(h, History::default());
        assert_matches!(problem, Some(Problem::UnknownVersion { version: 2, .. }));
        let recorded = record(dir.path(), t(NOW), |h| {
            h.frecency.record(&slug("api"), t(NOW));
        })
        .unwrap();
        assert_matches!(recorded, Some(Problem::UnknownVersion { .. }));
        assert_eq!(fs::read_to_string(frecency_file(dir.path())).unwrap(), body);
    }

    #[test]
    fn concurrent_recorders_lose_nothing() {
        const ROUNDS: u32 = 40;
        let dir = tempfile::tempdir().unwrap();
        std::thread::scope(|s| {
            for name in ["a", "b"] {
                let path = dir.path();
                s.spawn(move || {
                    for _ in 0..ROUNDS {
                        record(path, t(NOW), |h| h.frecency.record(&slug(name), t(NOW))).unwrap();
                    }
                });
            }
        });
        let (h, _) = load(dir.path());
        assert_eq!(count(&h, "a"), Some(ROUNDS));
        assert_eq!(count(&h, "b"), Some(ROUNDS));
    }
}
