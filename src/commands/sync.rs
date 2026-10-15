//! `pull <query>` and `sync [--jobs N]`.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::ExitCode;

use crate::app::App;
use crate::auth::{AuthError, EnvToken, Tokens};
use crate::cache::refresh::{Freshness, load_or_refresh};
use crate::cli::{ResolveArgs, SyncArgs};
use crate::commands::resolve::{
    ResolveError, is_cloned, record_access, resolve_target, short_name,
};
use crate::core::paths::repo_path;
use crate::core::sync_outcome::SyncOutcome;
use crate::error::Error;
use crate::sync::{SyncError, SyncTarget, failures, pull_one, sync_all};

/// Shown by the progress display until `sync_all` sends its header.
const SYNC_TITLE: &str = "Syncing repositories";

/// `pull <query>`: fetch and fast-forward one cloned repo, print its outcome,
/// record frecency. Never refreshes the cache.
///
/// # Errors
///
/// `CacheError::Missing`, resolution failures, `ResolveError::NotCloned`, or
/// `SyncError::PullFailed`.
///
/// # Panics
///
/// Only on a broken `Target` invariant (see
/// [`crate::commands::resolve::Target::repo`]).
pub fn pull(app: &mut App, args: &ResolveArgs) -> Result<ExitCode, Error> {
    let mut cache = load_or_refresh(app, Freshness::NoUpdate)?;
    let target = resolve_target(app, &mut cache, &args.query, args.pick)?;
    let slug = &target.repo(&cache.repos).slug;
    let name = short_name(slug, app.config.default_domain());
    if !is_cloned(&target.path) {
        return Err(ResolveError::NotCloned { repo: name }.into());
    }
    let domain = slug.domain();
    let tokens = Tokens::resolve(&*app.gh, &app.env.github_token, [domain]);
    let auth = tokens.git_auth(domain)?;
    app.ui.flush()?;
    let git = &*app.git;
    let path = target.path.as_path();
    let title = format!("pulling {name}");
    let outcome = app
        .ui
        .with_progress(&title, |_| pull_one(git, path, auth.as_ref(), &|_| {}));
    if let SyncOutcome::Error(detail) = outcome {
        return Err(SyncError::PullFailed { repo: name, detail }.into());
    }
    record_access(app, &mut cache, &target)?;
    app.ui.stdout_line(&format!("{name}: {outcome}"))?;
    Ok(ExitCode::SUCCESS)
}

/// `sync`: sync every repo that is in the cache and cloned at its expected
/// path, print one outcome line per repo sorted by relative path. Loads the
/// cache without refreshing it and records no frecency. Each repo's token is
/// resolved for its cached domain, as in `pull`. When no cached repo is
/// cloned, says so on stderr and succeeds.
///
/// # Errors
///
/// `AuthError::InvalidEnvToken` before anything runs, `CacheError::Missing`,
/// or `SyncError::SomeFailed` (after every line is printed).
pub fn sync(app: &mut App, args: &SyncArgs) -> Result<ExitCode, Error> {
    // Like `pull` and `clone`: an unusable `GITHUB_TOKEN` is a mistake to
    // report, not a reason to fetch every repo unauthenticated.
    if matches!(app.env.github_token, EnvToken::Invalid) {
        return Err(AuthError::InvalidEnvToken.into());
    }
    let cache = load_or_refresh(app, Freshness::NoUpdate)?;
    let work_dir = app.config.work_dir().to_path_buf();
    let default = app.config.default_domain();
    // `repo_path` under an empty root is the path relative to `work_dir`.
    let cloned: Vec<_> = cache
        .repos
        .iter()
        .map(|repo| {
            let rel = repo_path(Path::new(""), default, &repo.slug);
            (rel, repo.slug.domain())
        })
        .filter(|(rel, _)| is_cloned(&work_dir.join(rel)))
        .collect();
    if cloned.is_empty() {
        // Not an error, but say so: a mistyped `work_dir` or a changed
        // default domain would otherwise look like a successful sync.
        app.ui.stderr_line(&format!(
            "no cached repositories are cloned under {}",
            work_dir.display()
        ))?;
        return Ok(ExitCode::SUCCESS);
    }
    let domains: BTreeSet<_> = cloned.iter().map(|&(_, domain)| domain).collect();
    let tokens = Tokens::resolve(&*app.gh, &app.env.github_token, domains);
    let repos = cloned
        .into_iter()
        .map(|(path, domain)| {
            Ok(SyncTarget {
                path,
                auth: tokens.git_auth(domain)?,
            })
        })
        .collect::<Result<Vec<_>, AuthError>>()?;
    app.ui.flush()?;
    let git = &*app.git;
    let reports = app.ui.with_progress(SYNC_TITLE, |tasks| {
        sync_all(git, &work_dir, &repos, args.jobs, tasks)
    });
    for report in &reports {
        app.ui
            .stdout_line(&format!("{}: {}", report.path.display(), report.outcome))?;
    }
    failures(&reports).map_or(Ok(ExitCode::SUCCESS), |err| Err(err.into()))
}
