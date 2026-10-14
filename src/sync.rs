//! `pull` and `sync`: find cloned repos, fetch, and fast-forward when safe.
//!
//! `sync` never loads the cache; it walks the work directory.

pub mod run;
pub mod walk;

pub use run::{SyncReport, pull_one, sync_all};
pub use walk::find_repos;

use std::io;
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
    /// The work directory could not be walked.
    #[error("cannot read work directory {path:?}: {source}")]
    WorkDir {
        /// The directory that failed.
        path: PathBuf,
        /// The I/O error.
        source: io::Error,
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
