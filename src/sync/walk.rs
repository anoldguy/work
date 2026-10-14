//! Finding cloned repositories under the work directory.

use std::path::{Path, PathBuf};

use crate::sync::SyncError;

/// Every directory under `work_dir` that contains a `.git` (directory or
/// file), sorted. Does not descend into a repo once found, and skips
/// symlinks.
///
/// # Errors
///
/// [`SyncError::WorkDir`] when `work_dir` itself cannot be read; unreadable
/// subdirectories are skipped.
pub fn find_repos(work_dir: &Path) -> Result<Vec<PathBuf>, SyncError> {
    let _ = work_dir;
    unimplemented!("W1.10")
}
