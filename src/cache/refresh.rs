//! The freshness policy: load, maybe fetch, merge, save.

use std::time::Duration;

use crate::app::App;
use crate::cache::fetch::{FetchPlan, fetch_all};
use crate::cache::merge::{all_failed, merge};
use crate::cache::store::{self, LoadOutcome};
use crate::cache::{Cache, CacheError};
use crate::domain::{Domain, StoredTime};
use crate::text::plural;
use crate::ui::Ui;

/// Shown by the progress display until the fetch sends its header.
const PROGRESS_TITLE: &str = "Updating repositories";

/// How hard [`load_or_refresh`] tries to get fresh data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// Navigation, `list`, `favorite`: refresh when missing, corrupt, or
    /// stale. If every domain fails and a stale cache exists, warn and use
    /// it.
    AutoUpdate,
    /// `pull` and `sync`: load only; a missing or corrupt cache is
    /// [`CacheError::Missing`].
    NoUpdate,
    /// `update`: always refresh; on total failure, error and keep the
    /// existing file.
    Force {
        /// `--all-orgs`: ignore favourites.
        all_orgs: bool,
    },
}

/// Returns an owned cache. Fetching runs under `ui.with_progress`.
///
/// The merge starts from the copy read before the fetch and is written with
/// [`store::replace`]; a concurrent refresh's copy is simply overwritten
/// (both hold a complete listing). Frecency is never read or written here.
/// A corrupt file is renamed to `cache.json.corrupt` only once a refresh has
/// succeeded, immediately before the new cache is written, so a failed
/// refresh never touches the disk.
///
/// # Errors
///
/// [`CacheError::Missing`] under `NoUpdate`; [`CacheError::AllFailed`]
/// when a needed refresh fails for every domain and there is nothing to fall
/// back on (or under `Force`); store errors.
pub fn load_or_refresh(app: &App, ui: &mut Ui, freshness: Freshness) -> Result<Cache, CacheError> {
    let previous = match store::load(app.config.cache_dir()) {
        LoadOutcome::Loaded(cache) => Some(cache),
        LoadOutcome::Missing => None,
        LoadOutcome::Corrupt(reason) => {
            ui.warn(&format!("the repository cache is unreadable ({reason})"));
            None
        }
    };

    match (freshness, previous) {
        (Freshness::NoUpdate, previous) => previous.ok_or(CacheError::Missing),
        (Freshness::AutoUpdate, Some(cache)) if !cache.is_stale(app.now) => Ok(cache),
        (_, previous) => refresh(app, ui, freshness, previous),
    }
}

/// Fetches every effective domain, then merges and saves, or falls back.
fn refresh(
    app: &App,
    ui: &mut Ui,
    freshness: Freshness,
    previous: Option<Cache>,
) -> Result<Cache, CacheError> {
    let all_orgs = matches!(freshness, Freshness::Force { all_orgs: true });
    let plan = FetchPlan::from_config(&app.config, all_orgs);
    let domains: Vec<Domain> = plan.domains.iter().map(|d| d.domain.clone()).collect();
    let gh = &*app.gh;
    let outcomes = ui.with_progress(PROGRESS_TITLE, |tasks| fetch_all(&plan, gh, tasks));

    if all_failed(&outcomes) {
        return match (freshness, previous) {
            (Freshness::AutoUpdate, Some(stale)) => {
                ui.warn(&format!(
                    "could not refresh the repository cache; using the copy from {}",
                    describe_age(stale.updated_at, app.now)
                ));
                Ok(stale)
            }
            _ => Err(CacheError::AllFailed),
        };
    }

    let now = app.now;
    let filters = app.config.filters();
    let cache = merge(previous, outcomes, &domains, filters, now);
    store::replace(app.config.cache_dir(), &cache)?;
    Ok(cache)
}

/// `3 days ago`, `5 hours ago`, `12 minutes ago`, or `a future date`.
fn describe_age(updated_at: StoredTime, now: StoredTime) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    if updated_at.age_at(now) == Duration::ZERO && now.age_at(updated_at) > Duration::ZERO {
        return "a future date".to_owned();
    }
    let secs = updated_at.age_at(now).as_secs();
    let (n, one, many) = match secs {
        s if s >= DAY => (s / DAY, "day", "days"),
        s if s >= HOUR => (s / HOUR, "hour", "hours"),
        s => (s / MINUTE, "minute", "minutes"),
    };
    format!("{n} {} ago", plural(n, one, many))
}
