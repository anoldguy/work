//! The shared pipeline behind `cd`, `on`, `edit`, `clone`, `browse`, `open`
//! and `pull`: resolve a query, clone if needed, record the access.

use std::path::PathBuf;

use crate::app::App;
use crate::cache::Cache;
use crate::cli::ResolveArgs;
use crate::domain::{QueryKey, Slug};
use crate::error::Error;

/// A resolved repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// Index into `cache.repos`.
    pub index: usize,
    /// Where it lives (or will live) on disk.
    pub path: PathBuf,
}

/// What [`ensure_cloned`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloneStatus {
    /// `.git` already existed.
    Already,
    /// It was cloned now.
    Cloned,
}

/// Resolves `args` against `cache` (search, `decide`, then the picker on a
/// TTY), recording query frecency after an automatic answer or a pick.
///
/// # Errors
///
/// An invalid query; [`ResolveError::NotFound`]; [`ResolveError::Ambiguous`]
/// without a TTY or when the picker is cancelled; `UiError::NotInteractive`
/// for `--pick` without a TTY; `UiError::Interrupted` on Ctrl-C.
pub fn resolve_target(
    app: &mut App,
    cache: &mut Cache,
    args: &ResolveArgs,
) -> Result<Target, Error> {
    let _ = (app, cache, args);
    unimplemented!("W3.3")
}

/// Clones the target unless `.git` exists: creates the parent directories,
/// resolves a token for the repo's domain, and runs `git clone` under a
/// progress title.
///
/// # Errors
///
/// [`Error::CreateDir`], or the git failure.
pub fn ensure_cloned(app: &mut App, cache: &Cache, target: &Target) -> Result<CloneStatus, Error> {
    let _ = (app, cache, target);
    unimplemented!("W3.3")
}

/// Records repo frecency for the target and saves the cache. Called once the
/// command's action has succeeded.
///
/// # Errors
///
/// Saving the cache failed.
pub fn record_access(app: &App, cache: &mut Cache, target: &Target) -> Result<(), Error> {
    let _ = (app, cache, target);
    unimplemented!("W3.3")
}

/// A resolution failure.
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    /// No repo matched.
    #[error("repository not found: {query}")]
    NotFound {
        /// The normalised query.
        query: QueryKey,
    },
    /// Several repos matched and nobody picked. A list error: `candidates`
    /// are printed as `  - <slug>` lines after the summary.
    #[error("ambiguous query '{query}' (use a more specific query, or run in a terminal to pick)")]
    Ambiguous {
        /// The normalised query.
        query: QueryKey,
        /// Every candidate (at most `RESULT_LIMIT`), in rank order.
        candidates: Vec<Slug>,
    },
    /// `pull` on a repo that has not been cloned.
    #[error("{repo} is not cloned (run 'work clone {repo}')")]
    NotCloned {
        /// The repo as shown to the user, e.g. `acme/foo`.
        repo: String,
    },
}
