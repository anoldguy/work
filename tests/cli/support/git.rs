//! A bare origin repository built with real `git`, reachable over `file://`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Runs `git <args>` in `dir` with a hermetic identity and config, returning
/// trimmed stdout. Panics with git's stderr on failure.
pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?} failed in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A bare `origin.git` plus a private seed clone used to advance it.
///
/// The default branch is `main` and holds one commit (`README.md`).
#[derive(Debug)]
pub struct Origin {
    bare: PathBuf,
    seed: PathBuf,
}

impl Origin {
    /// Creates `<root>/origin.git` and `<root>/seed`, with one seed commit
    /// pushed to `main`.
    pub(super) fn create(root: &Path) -> Self {
        let bare = root.join("origin.git");
        let seed = root.join("seed");
        fs::create_dir_all(&bare).expect("create bare dir");
        fs::create_dir_all(&seed).expect("create seed dir");
        git(&bare, &["init", "--bare", "--initial-branch=main"]);
        git(&seed, &["init", "--initial-branch=main"]);
        git(&seed, &["remote", "add", "origin", &file_url(&bare)]);
        let origin = Self { bare, seed };
        origin.commit_file("README.md", "seed\n", "seed commit");
        origin
    }

    /// The `file://` URL to clone from.
    #[must_use]
    pub fn url(&self) -> String {
        file_url(&self.bare)
    }

    /// Path of the bare repository.
    #[must_use]
    pub fn bare_path(&self) -> &Path {
        &self.bare
    }

    /// Commits `content` to `file` in the seed clone and pushes `main`, so
    /// existing clones become behind. Returns the new commit hash.
    pub fn commit_file(&self, file: &str, content: &str, message: &str) -> String {
        fs::write(self.seed.join(file), content).expect("write file");
        git(&self.seed, &["add", "--", file]);
        git(&self.seed, &["commit", "-m", message]);
        git(&self.seed, &["push", "origin", "main"]);
        self.head()
    }

    /// The commit hash `main` points at in the origin.
    #[must_use]
    pub fn head(&self) -> String {
        git(&self.bare, &["rev-parse", "main"])
    }

    /// Clones the origin into `dest` (parents are created) and returns it.
    pub fn clone_to(&self, dest: &Path) -> PathBuf {
        let parent = dest.parent().expect("dest has a parent");
        fs::create_dir_all(parent).expect("create parents");
        git(
            parent,
            &["clone", &self.url(), dest.to_str().expect("utf-8 path")],
        );
        dest.to_path_buf()
    }
}

fn file_url(path: &Path) -> String {
    format!("file://{}", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_clones_over_file_and_fast_forwards() {
        let root = tempfile::tempdir().expect("tempdir");
        let origin = Origin::create(&root.path().join("o"));
        assert!(origin.url().starts_with("file:///"));
        assert!(origin.bare_path().ends_with("origin.git"));

        let clone = origin.clone_to(&root.path().join("c/work/repo"));
        assert_eq!(git(&clone, &["rev-parse", "HEAD"]), origin.head());

        let new = origin.commit_file("a.txt", "a\n", "second");
        assert_ne!(git(&clone, &["rev-parse", "HEAD"]), new);
        git(&clone, &["pull", "--ff-only"]);
        assert_eq!(git(&clone, &["rev-parse", "HEAD"]), new);
        assert_eq!(git(&clone, &["status", "--porcelain"]), "");
    }
}
