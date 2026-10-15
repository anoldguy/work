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
    /// line).
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

/// Atomically writes `cache` to `dir`, creating `dir` if needed.
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

/// Renames whatever is at `cache.json` to `cache.json.corrupt`, first
/// removing an earlier `cache.json.corrupt` of any kind so the rename cannot
/// trip over it.
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

/// Writes `cache` to `dir`, first renaming a file that does not load to
/// `cache.json.corrupt`. No lock: every writer holds a complete listing, so
/// the last one wins.
///
/// # Errors
///
/// [`CacheError::Write`] or [`CacheError::Encode`].
pub fn replace(dir: &Path, cache: &Cache) -> Result<(), CacheError> {
    if matches!(load(dir), LoadOutcome::Corrupt(_)) {
        quarantine(dir)?;
    }
    save(dir, cache)
}
