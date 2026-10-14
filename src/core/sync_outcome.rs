//! Turns git facts into a sync decision and outcome (pure).

use std::fmt;

use crate::text::plural;

/// The current branch's relation to its configured upstream (`@{u}`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upstream {
    /// No upstream configured (a local-only branch).
    None,
    /// Commits ahead of and behind the upstream.
    Tracking {
        /// Local commits not on the upstream.
        ahead: u32,
        /// Upstream commits not in the local branch.
        behind: u32,
    },
}

/// What one `git status --porcelain=v2 --branch` says about a repo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoFacts {
    /// HEAD is detached.
    Detached {
        /// Any entry line (including untracked files).
        dirty: bool,
    },
    /// HEAD is on a branch.
    Branch {
        /// Any entry line (including untracked files).
        dirty: bool,
        /// Relation to `@{u}`.
        upstream: Upstream,
    },
}

/// Why a repo was fetched but not fast-forwarded. None of these is an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchedReason {
    /// Uncommitted or untracked changes.
    DirtyWorktree,
    /// Local commits that are not on the upstream.
    Diverged,
    /// Not on a branch.
    DetachedHead,
    /// The branch has no upstream.
    NoUpstream,
}

impl fmt::Display for FetchedReason {
    /// `dirty worktree`, `diverged`, `detached HEAD`, or `no upstream`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::DirtyWorktree => "dirty worktree",
            Self::Diverged => "diverged",
            Self::DetachedHead => "detached HEAD",
            Self::NoUpstream => "no upstream",
        })
    }
}

/// The result of syncing one repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOutcome {
    /// Fast-forwarded by this many commits.
    Pulled {
        /// Commits fast-forwarded.
        commits: u32,
    },
    /// Clean and already at the upstream.
    UpToDate,
    /// Fetched only.
    Fetched(FetchedReason),
    /// Something failed; one line, already redacted.
    Error(String),
}

impl fmt::Display for SyncOutcome {
    /// The report text: `pulled (1 commit)` or `pulled (N commits)`, `up to date`,
    /// `fetched (<reason>)`, or `error: <message>`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pulled { commits } => {
                let noun = plural(*commits, "commit", "commits");
                write!(f, "pulled ({commits} {noun})")
            }
            Self::UpToDate => f.write_str("up to date"),
            Self::Fetched(reason) => write!(f, "fetched ({reason})"),
            Self::Error(message) => write!(f, "error: {message}"),
        }
    }
}

/// What to do after fetching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Run `git merge --ff-only @{u}`, which will pull this many commits.
    FastForward {
        /// Commits behind the upstream.
        commits: u32,
    },
    /// Nothing more to run; this is the outcome.
    Done(SyncOutcome),
}

/// Decides the next step. Precedence: dirty, then detached, then no
/// upstream, then `behind == 0` (up to date), then `ahead > 0` (diverged),
/// else fast-forward.
#[must_use]
pub fn classify(facts: &RepoFacts) -> Plan {
    let (dirty, upstream) = match *facts {
        RepoFacts::Detached { dirty } => (dirty, None),
        RepoFacts::Branch { dirty, upstream } => (dirty, Some(upstream)),
    };
    let fetched = |reason| Plan::Done(SyncOutcome::Fetched(reason));
    if dirty {
        return fetched(FetchedReason::DirtyWorktree);
    }
    match upstream {
        None => fetched(FetchedReason::DetachedHead),
        Some(Upstream::None) => fetched(FetchedReason::NoUpstream),
        Some(Upstream::Tracking { behind: 0, .. }) => Plan::Done(SyncOutcome::UpToDate),
        Some(Upstream::Tracking { ahead, .. }) if ahead > 0 => fetched(FetchedReason::Diverged),
        Some(Upstream::Tracking { behind, .. }) => Plan::FastForward { commits: behind },
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;

    fn branch(dirty: bool, upstream: Upstream) -> RepoFacts {
        RepoFacts::Branch { dirty, upstream }
    }

    fn tracking(ahead: u32, behind: u32) -> Upstream {
        Upstream::Tracking { ahead, behind }
    }

    #[test]
    fn classify_truth_table() {
        let fetched = |r| Plan::Done(SyncOutcome::Fetched(r));
        let cases = [
            (
                branch(true, tracking(0, 3)),
                fetched(FetchedReason::DirtyWorktree),
            ),
            (
                branch(true, Upstream::None),
                fetched(FetchedReason::DirtyWorktree),
            ),
            (
                RepoFacts::Detached { dirty: true },
                fetched(FetchedReason::DirtyWorktree),
            ),
            (
                RepoFacts::Detached { dirty: false },
                fetched(FetchedReason::DetachedHead),
            ),
            (
                branch(false, Upstream::None),
                fetched(FetchedReason::NoUpstream),
            ),
            (
                branch(false, tracking(0, 0)),
                Plan::Done(SyncOutcome::UpToDate),
            ),
            (
                branch(false, tracking(2, 0)),
                Plan::Done(SyncOutcome::UpToDate),
            ),
            (
                branch(false, tracking(1, 4)),
                fetched(FetchedReason::Diverged),
            ),
            (
                branch(false, tracking(0, 4)),
                Plan::FastForward { commits: 4 },
            ),
        ];
        for (facts, expected) in cases {
            assert_eq!(classify(&facts), expected, "{facts:?}");
        }
    }

    #[test]
    fn display_texts() {
        assert_eq!(
            SyncOutcome::Pulled { commits: 3 }.to_string(),
            "pulled (3 commits)"
        );
        assert_eq!(
            SyncOutcome::Pulled { commits: 1 }.to_string(),
            "pulled (1 commit)"
        );
        assert_eq!(SyncOutcome::UpToDate.to_string(), "up to date");
        assert_eq!(SyncOutcome::Error("boom".into()).to_string(), "error: boom");
        for (r, t) in [
            (FetchedReason::DirtyWorktree, "dirty worktree"),
            (FetchedReason::Diverged, "diverged"),
            (FetchedReason::DetachedHead, "detached HEAD"),
            (FetchedReason::NoUpstream, "no upstream"),
        ] {
            assert_eq!(
                SyncOutcome::Fetched(r).to_string(),
                format!("fetched ({t})")
            );
        }
    }

    #[test]
    fn fast_forward_carries_behind_count() {
        assert_matches!(
            classify(&branch(false, tracking(0, 1))),
            Plan::FastForward { commits: 1 }
        );
    }
}
