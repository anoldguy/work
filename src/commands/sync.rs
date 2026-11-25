//! `pull <query>` and `sync [--jobs N]`.

use std::process::ExitCode;

use crate::app::App;
use crate::cache::refresh::Freshness;
use crate::cli::{ResolveArgs, SyncArgs};
use crate::commands::resolve::{Reach, is_cloned, load_known, record_access, resolve_target};
use crate::error::Error;
use crate::sync::{SyncError, SyncOutcome, SyncTarget, failures, pull_one, sync_all};
use crate::ui::Ui;

const SYNC_TITLE: &str = "Syncing repositories";

/// Fetches and fast-forwards one cloned repo, prints its outcome and records
/// frecency. Never refreshes the cache.
///
/// # Errors
///
/// `CacheError::Missing`, resolution failures, `SyncError::NotCloned`, or
/// `SyncError::PullFailed`.
pub fn pull(app: &App, ui: &mut Ui, args: &ResolveArgs) -> Result<ExitCode, Error> {
    let repos = load_known(app, ui, Freshness::NoUpdate)?;
    let target = resolve_target(app, ui, &repos, &args.query, args.pick, Reach::Known)?;
    let slug = &target.repo.slug;
    let name = app.config.short_name(slug);
    if !is_cloned(&target.path) {
        return Err(SyncError::NotCloned { repo: name }.into());
    }
    let domain = slug.domain();
    let path = target.path.as_path();
    let title = format!("pulling {name}");
    let outcome = ui.with_progress(&title, |_| pull_one(path, domain, &|_| {}));
    if let SyncOutcome::Error(detail) = outcome {
        return Err(SyncError::PullFailed { repo: name, detail }.into());
    }
    record_access(app, ui, &target);
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
    let known = load_known(app, ui, Freshness::NoUpdate)?;
    let work_dir = app.config.work_dir().to_path_buf();
    let repos: Vec<_> = known
        .iter()
        .map(|repo| SyncTarget {
            path: app.config.relative_repo_path(&repo.slug),
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
    let reports = ui.with_progress(SYNC_TITLE, |tasks| {
        sync_all(&work_dir, &repos, args.jobs, tasks)
    });
    for report in &reports {
        ui.stdout_line(&format!("{}: {}", report.path.display(), report.outcome))?;
    }
    failures(&reports).map_or(Ok(ExitCode::SUCCESS), |err| Err(err.into()))
}
