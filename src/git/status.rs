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

    use super::*;

    #[test]
    fn porcelain_v2_becomes_repo_facts() {
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
        let gone = "# branch.oid abc\n# branch.head side\n# branch.upstream origin/side\n";
        assert_matches!(
            parse_v2(gone),
            Ok(RepoFacts::Branch {
                dirty: false,
                upstream: Upstream::None
            })
        );
        assert_matches!(
            parse_v2("# branch.oid abc\n# branch.head (detached)\n"),
            Ok(RepoFacts::Detached { dirty: false })
        );
    }

    #[test]
    fn malformed_status_is_an_error() {
        assert_matches!(parse_v2(""), Err(GitError::Status { .. }));
        assert_matches!(parse_v2("? only\n"), Err(GitError::Status { .. }));
        assert_matches!(
            parse_v2("# branch.head main\n# branch.ab nope\n"),
            Err(GitError::Status { .. })
        );
    }
}
