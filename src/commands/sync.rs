//! `pull <query>` and `sync [--jobs N]`.

use std::path::Path;
use std::process::ExitCode;

use crate::app::App;
use crate::cache::refresh::{Freshness, load_or_refresh};
use crate::cli::{ResolveArgs, SyncArgs};
use crate::commands::resolve::{
    ResolveError, is_cloned, record_access, resolve_target, short_name,
};
use crate::core::paths::repo_path;
use crate::core::sync_outcome::SyncOutcome;
use crate::error::Error;
use crate::sync::{SyncError, SyncTarget, failures, pull_one, sync_all};
use crate::ui::Ui;

const SYNC_TITLE: &str = "Syncing repositories";

/// Fetches and fast-forwards one cloned repo, prints its outcome and records
/// frecency. Never refreshes the cache.
///
/// # Errors
///
/// `CacheError::Missing`, resolution failures, `ResolveError::NotCloned`, or
/// `SyncError::PullFailed`.
///
/// # Panics
///
/// Only on a broken `Target` invariant.
pub fn pull(app: &App, ui: &mut Ui, args: &ResolveArgs) -> Result<ExitCode, Error> {
    let cache = load_or_refresh(app, ui, Freshness::NoUpdate)?;
    let target = resolve_target(app, ui, &cache, &args.query, args.pick)?;
    let slug = &target.repo(&cache.repos).slug;
    let name = short_name(slug, app.config.default_domain());
    if !is_cloned(&target.path) {
        return Err(ResolveError::NotCloned { repo: name }.into());
    }
    let domain = slug.domain();
    let git = &*app.git;
    let path = target.path.as_path();
    let title = format!("pulling {name}");
    let outcome = ui.with_progress(&title, |_| pull_one(git, path, domain, &|_| {}));
    if let SyncOutcome::Error(detail) = outcome {
        return Err(SyncError::PullFailed { repo: name, detail }.into());
    }
    record_access(app, ui, &cache, &target);
    ui.stdout_line(&format!("{name}: {outcome}"))?;
    Ok(ExitCode::SUCCESS)
}

/// Syncs every cached repo cloned at its expected path, printing one outcome
/// line per repo sorted by relative path. Never refreshes the cache or
/// records frecency.
///
/// # Errors
///
/// `CacheError::Missing`, or `SyncError::SomeFailed` (after every line is
/// printed).
pub fn sync(app: &App, ui: &mut Ui, args: &SyncArgs) -> Result<ExitCode, Error> {
    let cache = load_or_refresh(app, ui, Freshness::NoUpdate)?;
    let work_dir = app.config.work_dir().to_path_buf();
    let default = app.config.default_domain();
    // `repo_path` under an empty root is the path relative to `work_dir`.
    let repos: Vec<_> = cache
        .repos
        .iter()
        .map(|repo| SyncTarget {
            path: repo_path(Path::new(""), default, &repo.slug),
            domain: repo.slug.domain(),
        })
        .filter(|target| is_cloned(&work_dir.join(&target.path)))
        .collect();
    if repos.is_empty() {
        // Not an error, but say so: a mistyped `work_dir` or a changed
        // default domain would otherwise look like a successful sync.
        ui.stderr_line(&format!(
            "no cached repositories are cloned under {}",
            work_dir.display()
        ))?;
        return Ok(ExitCode::SUCCESS);
    }
    let git = &*app.git;
    let reports = ui.with_progress(SYNC_TITLE, |tasks| {
        sync_all(git, &work_dir, &repos, args.jobs, tasks)
    });
    for report in &reports {
        ui.stdout_line(&format!("{}: {}", report.path.display(), report.outcome))?;
    }
    failures(&reports).map_or(Ok(ExitCode::SUCCESS), |err| Err(err.into()))
}
