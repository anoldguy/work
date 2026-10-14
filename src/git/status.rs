//! Parser for `git status --porcelain=v2 --branch` (pure).

use crate::core::sync_outcome::{RepoFacts, Upstream};
use crate::git::GitError;

fn bad(detail: &str) -> GitError {
    GitError::Status {
        detail: detail.to_owned(),
    }
}

/// Parses `+A -B` from a `# branch.ab` header value.
fn parse_ab(value: &str) -> Option<(u32, u32)> {
    let (a, b) = value.split_once(' ')?;
    Some((
        a.strip_prefix('+')?.parse().ok()?,
        b.strip_prefix('-')?.parse().ok()?,
    ))
}

/// Parses porcelain v2 output: `# branch.head` (a name or `(detached)`),
/// `# branch.upstream`, and `# branch.ab +A -B`; any entry line (including
/// `?` untracked) means dirty. A configured upstream that no longer exists
/// has no `branch.ab` line and is reported as [`Upstream::None`].
///
/// Callers must run `git status --porcelain=v2 --branch
/// --untracked-files=normal` so user config cannot hide untracked files.
///
/// # Errors
///
/// [`GitError::Status`] when the branch header is missing or malformed.
pub fn parse_v2(output: &str) -> Result<RepoFacts, GitError> {
    let mut head = None;
    let mut ab = None;
    let mut dirty = false;
    for line in output.lines().filter(|l| !l.is_empty()) {
        if let Some(header) = line.strip_prefix("# ") {
            if let Some(value) = header.strip_prefix("branch.head ") {
                head = Some(value);
            } else if let Some(value) = header.strip_prefix("branch.ab ") {
                ab = Some(parse_ab(value).ok_or_else(|| bad("malformed branch.ab header"))?);
            }
        } else {
            dirty = true;
        }
    }
    let head = head.ok_or_else(|| bad("missing branch.head header"))?;
    if head == "(detached)" {
        return Ok(RepoFacts::Detached { dirty });
    }
    let upstream = ab.map_or(Upstream::None, |(ahead, behind)| Upstream::Tracking {
        ahead,
        behind,
    });
    Ok(RepoFacts::Branch { dirty, upstream })
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::path::Path;
    use std::process::Command;

    use super::*;

    fn run_git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8(out.stdout).expect("utf8")
    }

    fn git(dir: &Path, args: &[&str]) {
        run_git(dir, args);
    }

    fn status(dir: &Path) -> String {
        run_git(
            dir,
            &[
                "status",
                "--porcelain=v2",
                "--branch",
                "--untracked-files=normal",
            ],
        )
    }

    fn commit(dir: &Path, file: &str) {
        std::fs::write(dir.join(file), file).unwrap();
        git(dir, &["add", file]);
        git(dir, &["commit", "-q", "-m", file]);
    }

    #[test]
    fn real_git_tracking_states() {
        let tmp = tempfile::tempdir().unwrap();
        let up = tmp.path().join("up");
        let local = tmp.path().join("local");
        std::fs::create_dir(&up).unwrap();
        git(&up, &["init", "-q", "-b", "main"]);
        commit(&up, "a");
        git(tmp.path(), &["clone", "-q", up.to_str().unwrap(), "local"]);

        assert_matches!(
            parse_v2(&status(&local)),
            Ok(RepoFacts::Branch {
                dirty: false,
                upstream: Upstream::Tracking {
                    ahead: 0,
                    behind: 0
                }
            })
        );

        commit(&local, "b");
        assert_matches!(
            parse_v2(&status(&local)),
            Ok(RepoFacts::Branch {
                dirty: false,
                upstream: Upstream::Tracking {
                    ahead: 1,
                    behind: 0
                }
            })
        );

        commit(&up, "c");
        git(&local, &["fetch", "-q"]);
        assert_matches!(
            parse_v2(&status(&local)),
            Ok(RepoFacts::Branch {
                dirty: false,
                upstream: Upstream::Tracking {
                    ahead: 1,
                    behind: 1
                }
            })
        );

        std::fs::write(local.join("u"), "x").unwrap();
        assert_matches!(
            parse_v2(&status(&local)),
            Ok(RepoFacts::Branch { dirty: true, .. })
        );
        std::fs::remove_file(local.join("u")).unwrap();

        std::fs::write(local.join("a"), "changed").unwrap();
        git(&local, &["add", "a"]);
        assert_matches!(
            parse_v2(&status(&local)),
            Ok(RepoFacts::Branch { dirty: true, .. })
        );
    }

    #[test]
    fn real_git_gone_upstream_is_no_upstream() {
        let tmp = tempfile::tempdir().unwrap();
        let up = tmp.path().join("up");
        let local = tmp.path().join("local");
        std::fs::create_dir(&up).unwrap();
        git(&up, &["init", "-q", "-b", "main"]);
        commit(&up, "a");
        git(&up, &["branch", "side"]);
        git(tmp.path(), &["clone", "-q", up.to_str().unwrap(), "local"]);
        git(&local, &["checkout", "-q", "side"]);
        git(&up, &["branch", "-D", "side"]);
        git(&local, &["fetch", "-q", "--prune"]);
        let out = status(&local);
        assert!(out.contains("# branch.upstream origin/side"), "{out}");
        assert!(!out.contains("branch.ab"), "{out}");
        assert_matches!(
            parse_v2(&out),
            Ok(RepoFacts::Branch {
                dirty: false,
                upstream: Upstream::None
            })
        );
    }

    #[test]
    fn real_git_no_upstream_and_detached() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        git(d, &["init", "-q", "-b", "main"]);
        assert_matches!(
            parse_v2(&status(d)),
            Ok(RepoFacts::Branch {
                dirty: false,
                upstream: Upstream::None
            })
        );
        commit(d, "a");
        git(d, &["checkout", "-q", "--detach"]);
        assert_matches!(
            parse_v2(&status(d)),
            Ok(RepoFacts::Detached { dirty: false })
        );
        std::fs::write(d.join("z"), "z").unwrap();
        assert_matches!(
            parse_v2(&status(d)),
            Ok(RepoFacts::Detached { dirty: true })
        );
    }

    #[test]
    fn literal_outputs() {
        let tracked = "# branch.oid abc\n# branch.head main\n# branch.upstream o/main\n# branch.ab +2 -5\n1 .M N... 100644 100644 100644 a b f\n";
        assert_matches!(
            parse_v2(tracked),
            Ok(RepoFacts::Branch {
                dirty: true,
                upstream: Upstream::Tracking {
                    ahead: 2,
                    behind: 5
                }
            })
        );
        assert_matches!(
            parse_v2("# branch.oid (initial)\n# branch.head main\n? new\n"),
            Ok(RepoFacts::Branch {
                dirty: true,
                upstream: Upstream::None
            })
        );
        assert_matches!(
            parse_v2("# branch.oid abc\n# branch.head (detached)\n"),
            Ok(RepoFacts::Detached { dirty: false })
        );
    }

    #[test]
    fn malformed_is_error() {
        assert_matches!(parse_v2(""), Err(GitError::Status { .. }));
        assert_matches!(parse_v2("? only\n"), Err(GitError::Status { .. }));
        assert_matches!(
            parse_v2("# branch.head main\n# branch.ab nope\n"),
            Err(GitError::Status { .. })
        );
    }
}
