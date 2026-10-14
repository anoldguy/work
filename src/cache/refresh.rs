//! The freshness policy: load, maybe fetch, merge, save.

use crate::app::App;
use crate::cache::{Cache, CacheError};

/// How hard [`load_or_refresh`] tries to get fresh data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// Navigation, `list`, `favorite`: refresh when missing, corrupt, or
    /// stale. If every domain fails and a stale cache exists, warn and use
    /// it.
    AutoUpdate,
    /// `pull`: load only; a missing cache is [`CacheError::Missing`].
    NoUpdate,
    /// `update`: always refresh; on total failure, error and keep the
    /// existing file.
    Force {
        /// `--all-orgs`: ignore favourites.
        all_orgs: bool,
    },
}

/// Returns an owned cache, held by the command separately from `app` so both
/// can be borrowed mutably. A corrupt file is quarantined before a rebuild.
/// Fetching runs under `app.ui.with_progress`.
///
/// # Errors
///
/// [`CacheError::Missing`] under `NoUpdate`; [`CacheError::AllFailed`] when a
/// needed refresh fails for every domain and there is nothing to fall back
/// on (or under `Force`); store errors.
pub fn load_or_refresh(app: &mut App, freshness: Freshness) -> Result<Cache, CacheError> {
    let _ = (app, freshness);
    unimplemented!("W4.1")
}
