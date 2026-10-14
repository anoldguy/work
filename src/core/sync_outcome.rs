//! Turns git facts into a sync decision and outcome (pure).

use std::fmt;

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
        let _ = (self, f);
        unimplemented!("W1.10")
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
    /// The report text: `pulled (N commits)`, `up to date`,
    /// `fetched (<reason>)`, or `error: <message>`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = (self, f);
        unimplemented!("W1.10")
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
    let _ = facts;
    unimplemented!("W1.10")
}
