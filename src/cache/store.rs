//! Reading and atomically writing `cache.json`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::cache::{CACHE_FILE, CACHE_VERSION, Cache, CacheError};
use crate::fsutil::atomic_write;
use crate::text::one_line;

/// What reading the cache found.
#[derive(Debug)]
pub enum LoadOutcome {
    /// A readable cache of the current version.
    Loaded(Cache),
    /// No cache file.
    Missing,
    /// Unreadable, unparsable, or another version; the string says why (one
    /// line). The caller quarantines it and rebuilds.
    Corrupt(String),
}

/// Just the version, read first so another version reports as such.
#[derive(serde::Deserialize)]
struct Peek {
    version: u32,
}

/// Leftover temporary files older than this are deleted on save.
const STALE_TEMP_AGE: Duration = Duration::from_hours(1);

/// `{dir}/cache.json`.
#[must_use]
pub fn cache_file(dir: &Path) -> PathBuf {
    dir.join(CACHE_FILE)
}

/// Reads the cache in `dir`. Never fails: every problem is a
/// [`LoadOutcome`].
#[must_use]
pub fn load(dir: &Path) -> LoadOutcome {
    let path = cache_file(dir);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return LoadOutcome::Missing,
        Err(e) => return LoadOutcome::Corrupt(one_line(&format!("cannot read file: {e}"))),
    };
    if let Ok(Peek { version }) = serde_json::from_slice(&bytes)
        && version != CACHE_VERSION
    {
        return LoadOutcome::Corrupt(format!(
            "unsupported cache version {version} (expected {CACHE_VERSION})"
        ));
    }
    match serde_json::from_slice::<Cache>(&bytes) {
        Ok(cache) => LoadOutcome::Loaded(cache),
        Err(e) => LoadOutcome::Corrupt(one_line(&format!("invalid cache: {e}"))),
    }
}

/// Writes `cache` to `dir` atomically: a temporary file in `dir`, fsync,
/// then rename. Creates `dir` if needed. Concurrent writers: last one wins.
///
/// # Errors
///
/// [`CacheError::Write`] or [`CacheError::Encode`].
pub fn save(dir: &Path, cache: &Cache) -> Result<(), CacheError> {
    let mut json =
        serde_json::to_vec_pretty(cache).map_err(|source| CacheError::Encode { source })?;
    json.push(b'\n');
    let target = cache_file(dir);
    atomic_write(&target, &json).map_err(|source| CacheError::Write {
        path: target,
        source,
    })?;
    remove_old_temp_files(dir);
    Ok(())
}

/// Best-effort removal of `cache.json.tmp-*` files a crashed writer left
/// behind (older than [`STALE_TEMP_AGE`]).
fn remove_old_temp_files(dir: &Path) {
    let prefix = format!("{CACHE_FILE}.tmp-");
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry.file_name().to_string_lossy().starts_with(&prefix)
            && entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > STALE_TEMP_AGE);
        if old {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Renames a corrupt `cache.json` (a file, or anything else at that path)
/// to `cache.json.corrupt` before a rebuild, first removing an earlier
/// `cache.json.corrupt` of any kind so the rename cannot trip over it.
///
/// # Errors
///
/// [`CacheError::Write`] when the rename fails.
pub fn quarantine(dir: &Path) -> Result<(), CacheError> {
    let from = cache_file(dir);
    let to = dir.join(format!("{CACHE_FILE}.corrupt"));
    match fs::symlink_metadata(&to) {
        Ok(meta) if meta.is_dir() => {
            let _ = fs::remove_dir_all(&to);
        }
        Ok(_) => {
            let _ = fs::remove_file(&to);
        }
        Err(_) => {}
    }
    fs::rename(&from, &to).map_err(|source| CacheError::Write { path: from, source })
}

/// `{dir}/cache.json.lock`: the advisory lock [`update`] holds.
fn lock_file(dir: &Path) -> PathBuf {
    dir.join(format!("{CACHE_FILE}.lock"))
}

/// Takes the exclusive advisory lock on `{dir}/cache.json.lock`; it is
/// released when the returned file is dropped. Best effort: `None` when the
/// lock file cannot be created or the filesystem cannot lock, and the caller
/// proceeds unlocked (the write itself is still atomic).
fn lock(dir: &Path) -> Option<fs::File> {
    fs::create_dir_all(dir).ok()?;
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_file(dir))
        .ok()?;
    file.lock().ok()?;
    Some(file)
}

/// Read-modify-write under the cache lock, so concurrent `work` processes
/// never lose each other's changes.
///
/// Takes the advisory lock, reloads the newest cache from disk (`None` when
/// it is missing or corrupt), applies `f`, quarantines a corrupt file,
/// writes the result atomically, releases the lock, and returns what was
/// written. Keep slow work (network, clones) out of `f`.
///
/// # Errors
///
/// [`CacheError::Write`] or [`CacheError::Encode`] (from the quarantine or
/// the save).
pub fn update(dir: &Path, f: impl FnOnce(Option<Cache>) -> Cache) -> Result<Cache, CacheError> {
    let _lock = lock(dir);
    let (latest, corrupt) = match load(dir) {
        LoadOutcome::Loaded(cache) => (Some(cache), false),
        LoadOutcome::Missing => (None, false),
        LoadOutcome::Corrupt(_) => (None, true),
    };
    let cache = f(latest);
    if corrupt {
        quarantine(dir)?;
    }
    save(dir, &cache)?;
    Ok(cache)
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use proptest::prelude::*;

    use super::*;
    use crate::domain::{Repo, StoredTime};

    fn repo(name: &str) -> Repo {
        serde_json::from_value(serde_json::json!({
            "slug": format!("github.com/acme/{name}"),
            "clone_url": format!("https://github.com/acme/{name}.git"),
            "html_url": format!("https://github.com/acme/{name}"),
        }))
        .unwrap()
    }

    fn t(secs: i64) -> StoredTime {
        StoredTime::from_unix_seconds(secs).unwrap()
    }

    #[test]
    fn load_reports_missing_when_no_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_matches!(load(dir.path()), LoadOutcome::Missing);
        assert_matches!(load(&dir.path().join("nope")), LoadOutcome::Missing);
    }

    #[test]
    fn save_creates_parents_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("a/b");
        let cache = Cache::new(vec![repo("one"), repo("two")], t(1_700_000_000));
        save(&sub, &cache).unwrap();
        assert_matches!(load(&sub), LoadOutcome::Loaded(c) if c == cache);
    }

    #[test]
    fn save_leaves_no_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), &Cache::new(vec![], t(0))).unwrap();
        save(dir.path(), &Cache::new(vec![], t(1))).unwrap();
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, [std::ffi::OsString::from(CACHE_FILE)]);
    }

    #[test]
    fn save_reports_write_error_when_dir_is_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        fs::write(&file, "x").unwrap();
        assert_matches!(
            save(&file, &Cache::new(vec![], t(0))),
            Err(CacheError::Write { .. })
        );
    }

    #[test]
    fn invalid_json_is_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(cache_file(dir.path()), "{not json").unwrap();
        assert_matches!(load(dir.path()), LoadOutcome::Corrupt(r) if !r.contains('\n'));
    }

    #[test]
    fn unknown_shape_is_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(cache_file(dir.path()), r#"{"hello":"world"}"#).unwrap();
        assert_matches!(load(dir.path()), LoadOutcome::Corrupt(_));
        fs::write(cache_file(dir.path()), "[]").unwrap();
        assert_matches!(load(dir.path()), LoadOutcome::Corrupt(_));
    }

    #[test]
    fn other_version_is_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            cache_file(dir.path()),
            r#"{"version":99,"updated_at":"2025-01-01T00:00:00Z","repos":[]}"#,
        )
        .unwrap();
        assert_matches!(load(dir.path()), LoadOutcome::Corrupt(r) if r.contains("99"));
    }

    #[test]
    fn missing_version_is_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            cache_file(dir.path()),
            r#"{"updated_at":"2025-01-01T00:00:00Z","repos":[]}"#,
        )
        .unwrap();
        assert_matches!(load(dir.path()), LoadOutcome::Corrupt(_));
    }

    #[test]
    fn unreadable_path_is_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(cache_file(dir.path())).unwrap();
        assert_matches!(load(dir.path()), LoadOutcome::Corrupt(_));
    }

    #[test]
    fn quarantine_renames_and_replaces_earlier_file() {
        let dir = tempfile::tempdir().unwrap();
        let corrupt = dir.path().join("cache.json.corrupt");
        fs::write(&corrupt, "old").unwrap();
        fs::write(cache_file(dir.path()), "new").unwrap();
        quarantine(dir.path()).unwrap();
        assert_matches!(load(dir.path()), LoadOutcome::Missing);
        assert_eq!(fs::read_to_string(&corrupt).unwrap(), "new");
    }

    #[test]
    fn quarantine_without_file_is_a_write_error() {
        let dir = tempfile::tempdir().unwrap();
        assert_matches!(quarantine(dir.path()), Err(CacheError::Write { .. }));
    }

    #[test]
    fn future_version_with_other_shape_reports_version() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(cache_file(dir.path()), r#"{"version":2,"repos":{"x":1}}"#).unwrap();
        assert_matches!(load(dir.path()), LoadOutcome::Corrupt(r) if r.contains("version 2"));
    }

    #[test]
    fn save_keeps_a_symlinked_cache_file() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.json");
        fs::write(&real, "{}").unwrap();
        let cdir = dir.path().join("c");
        fs::create_dir(&cdir).unwrap();
        std::os::unix::fs::symlink(&real, cache_file(&cdir)).unwrap();
        save(&cdir, &Cache::new(vec![], t(1))).unwrap();
        assert!(
            fs::symlink_metadata(cache_file(&cdir))
                .unwrap()
                .is_symlink()
        );
        assert_matches!(load(&cdir), LoadOutcome::Loaded(_));
    }

    #[test]
    fn save_keeps_recent_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let tmp = dir.path().join("cache.json.tmp-1-1");
        fs::write(&tmp, "x").unwrap();
        save(dir.path(), &Cache::new(vec![], t(1))).unwrap();
        assert!(tmp.exists());
    }

    #[test]
    fn quarantine_moves_a_directory_over_an_earlier_corrupt_file() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(cache_file(dir.path())).unwrap();
        fs::write(dir.path().join("cache.json.corrupt"), "old").unwrap();
        quarantine(dir.path()).unwrap();
        assert!(dir.path().join("cache.json.corrupt").is_dir());
        assert_matches!(load(dir.path()), LoadOutcome::Missing);
    }

    #[test]
    fn quarantine_replaces_an_earlier_corrupt_directory() {
        let dir = tempfile::tempdir().unwrap();
        let corrupt = dir.path().join("cache.json.corrupt");
        fs::create_dir_all(corrupt.join("inner")).unwrap();
        fs::write(cache_file(dir.path()), "new").unwrap();
        quarantine(dir.path()).unwrap();
        assert_eq!(fs::read_to_string(&corrupt).unwrap(), "new");
    }

    #[test]
    fn update_sees_the_latest_cache_and_saves_the_result() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), &Cache::new(vec![repo("one")], t(1))).unwrap();
        let written = update(dir.path(), |latest| {
            let mut cache = latest.expect("loaded");
            cache.repos.push(repo("two"));
            cache
        })
        .unwrap();
        assert_eq!(written.repos.len(), 2);
        assert_matches!(load(dir.path()), LoadOutcome::Loaded(c) if c == written);
    }

    #[test]
    fn update_starts_from_none_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("a");
        let mut seen = None;
        update(&sub, |latest| {
            seen = Some(latest.is_none());
            Cache::new(vec![], t(1))
        })
        .unwrap();
        assert_eq!(seen, Some(true));
        assert_matches!(load(&sub), LoadOutcome::Loaded(_));
    }

    #[test]
    fn update_quarantines_a_corrupt_file_before_writing() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(cache_file(dir.path()), "{bad").unwrap();
        update(dir.path(), |latest| {
            assert!(latest.is_none());
            Cache::new(vec![], t(1))
        })
        .unwrap();
        let corrupt = dir.path().join("cache.json.corrupt");
        assert_eq!(fs::read_to_string(corrupt).unwrap(), "{bad");
        assert_matches!(load(dir.path()), LoadOutcome::Loaded(_));
    }

    #[test]
    fn interleaved_writers_lose_nothing() {
        const ROUNDS: u32 = 40;
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), &Cache::new(vec![repo("a"), repo("b")], t(1))).unwrap();
        let slug = |name: &str| repo(name).slug;
        std::thread::scope(|s| {
            for name in ["a", "b"] {
                let path = dir.path();
                s.spawn(move || {
                    for _ in 0..ROUNDS {
                        update(path, |latest| {
                            let mut cache = latest.expect("loaded");
                            cache.frecency.record(&slug(name), t(2));
                            cache
                        })
                        .unwrap();
                    }
                });
            }
        });
        let LoadOutcome::Loaded(cache) = load(dir.path()) else {
            panic!("cache should load");
        };
        for name in ["a", "b"] {
            assert_eq!(
                cache.frecency.get(&slug(name)).map(|d| d.access_count),
                Some(ROUNDS)
            );
        }
    }

    #[test]
    fn update_leaves_only_the_cache_and_its_lock() {
        let dir = tempfile::tempdir().unwrap();
        update(dir.path(), |_| Cache::new(vec![], t(1))).unwrap();
        let mut names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, ["cache.json", "cache.json.lock"]);
    }

    proptest! {
        #[test]
        fn round_trips_any_repo_list(
            names in proptest::collection::vec("[a-z][a-z0-9-]{0,12}", 0..8),
            secs in 0_i64..4_000_000_000,
        ) {
            let dir = tempfile::tempdir().unwrap();
            let cache = Cache::new(names.iter().map(|n| repo(n)).collect(), t(secs));
            save(dir.path(), &cache).unwrap();
            assert_matches!(load(dir.path()), LoadOutcome::Loaded(c) if c == cache);
        }
    }
}
