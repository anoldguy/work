//! `pull <query>` and `sync [--jobs N]`.

use std::path::PathBuf;
use std::process::ExitCode;
use std::slice;

use crate::app::App;
use crate::auth::{AuthError, GitAuth, Tokens};
use crate::cache::refresh::{Freshness, load_or_refresh};
use crate::cli::{ResolveArgs, SyncArgs};
use crate::commands::resolve::{ResolveError, record_access, resolve_target, short_name};
use crate::core::sync_outcome::SyncOutcome;
use crate::error::Error;
use crate::sync::{SyncError, failures, find_repos, pull_one, sync_all};

/// Shown by the progress display until `sync_all` sends its header.
const SYNC_TITLE: &str = "Syncing repositories";

/// `pull <query>`: fetch and fast-forward one cloned repo, print its outcome,
/// record frecency. Never refreshes the cache.
///
/// # Errors
///
/// `CacheError::Missing`, resolution failures, `ResolveError::NotCloned`, or
/// `SyncError::PullFailed`.
pub fn pull(app: &mut App, args: ResolveArgs) -> Result<ExitCode, Error> {
    let mut cache = load_or_refresh(app, Freshness::NoUpdate)?;
    let target = resolve_target(app, &mut cache, &args)?;
    drop(args);
    let slug = &cache.repos[target.index].slug;
    let name = short_name(slug, app.config.default_domain());
    if !target.path.join(".git").exists() {
        return Err(ResolveError::NotCloned { repo: name }.into());
    }
    let domain = slug.domain();
    let tokens = Tokens::resolve(&*app.gh, &app.env.github_token, slice::from_ref(domain));
    let auth = match tokens.get(domain) {
        Ok((token, _)) => Some(GitAuth::new(domain.clone(), token)),
        Err(AuthError::InvalidEnvToken) => return Err(AuthError::InvalidEnvToken.into()),
        Err(_) => None,
    };
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

/// `sync`: walk the work directory, sync every repo, print one outcome line
/// per repo sorted by relative path. Never loads the cache.
///
/// # Errors
///
/// `SyncError::WorkDir`, or `SyncError::SomeFailed` (after every line is
/// printed).
#[expect(
    clippy::needless_pass_by_value,
    reason = "SyncArgs has only Copy fields; the signature is fixed by the dispatcher"
)]
pub fn sync(app: &mut App, args: SyncArgs) -> Result<ExitCode, Error> {
    let work_dir = app.config.work_dir().to_path_buf();
    // `find_repos` returns paths under `work_dir`; `sync_all` wants them relative.
    let repos: Vec<PathBuf> = find_repos(&work_dir)?
        .into_iter()
        .map(|p| match p.strip_prefix(&work_dir) {
            Ok(rel) => rel.to_path_buf(),
            Err(_) => p,
        })
        .collect();
    let configured = app.config.effective_domains();
    let tokens = Tokens::resolve(&*app.gh, &app.env.github_token, &configured);
    app.ui.flush()?;
    let git = &*app.git;
    let reports = app.ui.with_progress(SYNC_TITLE, |sink| {
        sync_all(
            git,
            &work_dir,
            &repos,
            &tokens,
            &configured,
            args.jobs,
            sink,
        )
    });
    for report in &reports {
        app.ui
            .stdout_line(&format!("{}: {}", report.path.display(), report.outcome))?;
    }
    failures(&reports).map_or(Ok(ExitCode::SUCCESS), |err| Err(err.into()))
}
