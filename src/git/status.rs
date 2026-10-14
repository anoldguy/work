//! Parser for `git status --porcelain=v2 --branch` (pure).

use crate::core::sync_outcome::RepoFacts;
use crate::git::GitError;

/// Parses porcelain v2 output: `# branch.head` (a name or `(detached)`),
/// `# branch.upstream`, and `# branch.ab +A -B`; any entry line (including
/// `?` untracked) means dirty.
///
/// # Errors
///
/// [`GitError::Status`] when the branch header is missing or malformed.
pub fn parse_v2(output: &str) -> Result<RepoFacts, GitError> {
    let _ = output;
    unimplemented!("W1.10")
}
