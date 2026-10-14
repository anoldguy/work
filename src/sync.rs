//! `pull` and `sync`: fetch cloned repos and fast-forward when safe.
//!
//! Both act only on repos that are in the cache and cloned at their expected
//! path; `sync` never walks the work directory.

pub mod run;

pub use run::{SyncReport, SyncTarget, failures, pull_one, sync_all};

use std::path::PathBuf;

/// A sync failure.
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    /// Some repos ended in `SyncOutcome::Error`. A list error: `failed` is
    /// printed as `  - <path>` lines after the summary.
    #[error("{} of {total} repositories failed to sync", .failed.len())]
    SomeFailed {
        /// Paths relative to `work_dir`, sorted.
        failed: Vec<PathBuf>,
        /// Repos attempted.
        total: usize,
    },
    /// `pull` ended in an error outcome.
    #[error("could not pull {repo}: {detail}")]
    PullFailed {
        /// The repo as shown to the user, e.g. `acme/foo`.
        repo: String,
        /// The outcome's message.
        detail: String,
    },
}
