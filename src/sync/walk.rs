//! Finding cloned repositories under the work directory.

use std::fs;
use std::path::{Path, PathBuf};

use crate::sync::SyncError;

/// Deepest level a repo can sit at: `{domain}/{org}/{repo}`.
const MAX_DEPTH: usize = 3;

/// Every directory under `work_dir` that contains a `.git` (directory or
/// file), sorted. Does not descend into a repo once found, and skips
/// symlinks.
///
/// # Errors
///
/// [`SyncError::WorkDir`] when `work_dir` itself cannot be read; unreadable
/// subdirectories are skipped.
pub fn find_repos(work_dir: &Path) -> Result<Vec<PathBuf>, SyncError> {
    let root = fs::read_dir(work_dir).map_err(|source| SyncError::WorkDir {
        path: work_dir.to_path_buf(),
        source,
    })?;
    let mut repos = Vec::new();
    let mut stack: Vec<(PathBuf, usize)> = subdirs(root).into_iter().map(|d| (d, 1)).collect();
    while let Some((dir, depth)) = stack.pop() {
        if dir.join(".git").symlink_metadata().is_ok() {
            repos.push(dir);
        } else if depth < MAX_DEPTH
            && let Ok(entries) = fs::read_dir(&dir)
        {
            stack.extend(subdirs(entries).into_iter().map(|d| (d, depth + 1)));
        }
    }
    repos.sort();
    Ok(repos)
}

/// Child directories (not symlinks) of a directory listing.
fn subdirs(entries: fs::ReadDir) -> Vec<PathBuf> {
    entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;

    #[test]
    fn finds_nested_layout_and_stops_at_repos() {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path();
        fs::create_dir_all(r.join("gh/org/a/.git")).unwrap();
        fs::create_dir_all(r.join("gh/org/a/vendor/inner/.git")).unwrap();
        fs::create_dir_all(r.join("gh/org/b")).unwrap();
        fs::write(r.join("gh/org/b/.git"), "gitdir: x").unwrap();
        fs::create_dir_all(r.join("gh/org/empty")).unwrap();
        fs::write(r.join("loose.txt"), "x").unwrap();
        let repos = find_repos(r).unwrap();
        assert_eq!(repos, vec![r.join("gh/org/a"), r.join("gh/org/b")]);
    }

    #[test]
    fn does_not_descend_deeper_than_domain_org_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path();
        fs::create_dir_all(r.join("a/b/c/.git")).unwrap();
        fs::create_dir_all(r.join("a/b/c2/d/e/.git")).unwrap();
        assert_eq!(find_repos(r).unwrap(), vec![r.join("a/b/c")]);
    }

    #[cfg(unix)]
    #[test]
    fn skips_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path().join("work");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(r.join("real/.git")).unwrap();
        fs::create_dir_all(outside.join(".git")).unwrap();
        std::os::unix::fs::symlink(&outside, r.join("link")).unwrap();
        assert_eq!(find_repos(&r).unwrap(), vec![r.join("real")]);
    }

    #[test]
    fn missing_work_dir_errors() {
        let tmp = tempfile::tempdir().unwrap();
        assert_matches!(
            find_repos(&tmp.path().join("nope")),
            Err(SyncError::WorkDir { .. })
        );
    }
}
