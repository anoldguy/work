//! Reading and atomically writing `cache.json`.

use std::path::{Path, PathBuf};

use crate::cache::{CACHE_FILE, Cache, CacheError};

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

/// `{dir}/cache.json`.
#[must_use]
pub fn cache_file(dir: &Path) -> PathBuf {
    dir.join(CACHE_FILE)
}

/// Reads the cache in `dir`. Never fails: every problem is a
/// [`LoadOutcome`].
#[must_use]
pub fn load(dir: &Path) -> LoadOutcome {
    let _ = dir;
    unimplemented!("W2.5")
}

/// Writes `cache` to `dir` atomically: a temporary file in `dir`, fsync,
/// then rename. Creates `dir` if needed. Concurrent writers: last one wins.
///
/// # Errors
///
/// [`CacheError::Write`] or [`CacheError::Encode`].
pub fn save(dir: &Path, cache: &Cache) -> Result<(), CacheError> {
    let _ = (dir, cache);
    unimplemented!("W2.5")
}

/// Renames a corrupt `cache.json` to `cache.json.corrupt` (replacing any
/// earlier one) before a rebuild.
///
/// # Errors
///
/// [`CacheError::Write`] when the rename fails.
pub fn quarantine(dir: &Path) -> Result<(), CacheError> {
    let _ = dir;
    unimplemented!("W2.5")
}
