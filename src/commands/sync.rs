//! `pull <query>` and `sync [--jobs N]`.

use std::process::ExitCode;

use crate::app::App;
use crate::cli::{ResolveArgs, SyncArgs};
use crate::error::Error;

/// `pull <query>`: fetch and fast-forward one cloned repo, print its outcome,
/// record frecency. Never refreshes the cache.
///
/// # Errors
///
/// `CacheError::Missing`, resolution failures, `ResolveError::NotCloned`, or
/// `SyncError::PullFailed`.
pub fn pull(app: &mut App, args: ResolveArgs) -> Result<ExitCode, Error> {
    let _ = (app, args);
    unimplemented!("W5.3")
}

/// `sync`: walk the work directory, sync every repo, print one outcome line
/// per repo sorted by relative path. Never loads the cache.
///
/// # Errors
///
/// `SyncError::WorkDir`, or `SyncError::SomeFailed` (after every line is
/// printed).
pub fn sync(app: &mut App, args: SyncArgs) -> Result<ExitCode, Error> {
    let _ = (app, args);
    unimplemented!("W5.3")
}
