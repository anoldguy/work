//! `cd`, `on`, `edit`, `clone`, `browse`, `open`, their `sh-*` forms, and
//! `sh-editor`.
//!
//! Every command that names a repo runs the shared pipeline: load the cache
//! (refreshing it when stale), resolve the query, clone when the command
//! needs a local copy, perform the action, and only then record the access
//! (so a failed clone, editor or opener records nothing).

use std::fs;
use std::process::ExitCode;

use crate::app::App;
use crate::cache::Cache;
use crate::cache::refresh::{Freshness, load_or_refresh};
use crate::cli::{CdArgs, ResolveArgs};
use crate::commands::resolve::{Target, ensure_cloned, record_access, resolve_target};
use crate::env::EnvSnapshot;
use crate::error::Error;
use crate::platform::{self, EditorCmd};
use crate::shell::{self, ShellError};
use crate::ui::Ui;

/// How `cd`/`on` were invoked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Invocation {
    /// `sh-cd`/`sh-on` from the wrapper: print the target path on stdout.
    Wrapped,
    /// Plain `cd`/`on`: fail with `ShellError::NotActive` without resolving.
    Direct,
}

/// Whether a command needs the repo on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Local {
    /// Clone it first when `.git` is missing.
    Clone,
    /// Never clone (`browse`).
    NotNeeded,
}

/// `cd [query]`: prints the repo's absolute path (the work directory,
/// created if missing, when there is no query) for the wrapper to `cd` to.
///
/// # Errors
///
/// Not wrapped, an unprintable path, resolution, clone, or cache failures.
pub fn cd(app: &App, ui: &mut Ui, args: &CdArgs, via: Invocation) -> Result<ExitCode, Error> {
    require_wrapper(app, via)?;
    let Some(query) = &args.query else {
        let dir = app.config.work_dir();
        // Check the path can be printed before creating anything.
        let line = shell::path_line(dir)?;
        fs::create_dir_all(dir).map_err(|source| Error::CreateDir {
            path: dir.to_path_buf(),
            source,
        })?;
        ui.stdout_line(&line)?;
        return Ok(ExitCode::SUCCESS);
    };
    wrapped_target(app, ui, query, args.pick)
}

/// `on <query>`: prints the repo's absolute path, like [`cd`]; the wrapper
/// changes to it and then runs `sh-editor` (see [`sh_editor`]).
///
/// # Errors
///
/// Not wrapped, an unprintable path, resolution, clone, or cache failures.
pub fn on(app: &App, ui: &mut Ui, args: &ResolveArgs, via: Invocation) -> Result<ExitCode, Error> {
    require_wrapper(app, via)?;
    wrapped_target(app, ui, &args.query, args.pick)
}

/// `sh-editor`: runs the editor on the current directory with the terminal
/// attached. Resolves nothing and records nothing, so it needs no config.
///
/// # Errors
///
/// The current directory is unreadable, or the editor fails.
pub fn sh_editor(env: &EnvSnapshot, ui: &mut Ui) -> Result<ExitCode, Error> {
    let dir = std::env::current_dir().map_err(|source| ShellError::CurrentDir { source })?;
    // The editor inherits stdout: pending output must land first.
    ui.flush()?;
    platform::spawn_editor(&EditorCmd::from_env(env), &dir)?;
    Ok(ExitCode::SUCCESS)
}

/// Resolves `query`, clones it when missing, records the access, and prints
/// the target's absolute path.
fn wrapped_target(app: &App, ui: &mut Ui, query: &str, pick: bool) -> Result<ExitCode, Error> {
    let (cache, target) = resolve(app, ui, query, pick)?;
    // Check the path can be printed before cloning anything.
    let line = shell::path_line(&target.path)?;
    ensure_cloned(app, ui, &cache, &target)?;
    record_access(app, ui, &cache, &target);
    ui.stdout_line(&line)?;
    Ok(ExitCode::SUCCESS)
}

/// `edit <query>`: clones if needed and runs the editor.
///
/// # Errors
///
/// Resolution, clone, editor, or cache failures.
pub fn edit(app: &App, ui: &mut Ui, args: &ResolveArgs) -> Result<ExitCode, Error> {
    let (cache, target) = prepare(app, ui, args, Local::Clone)?;
    // The editor inherits stdout: pending output must land first.
    ui.flush()?;
    platform::spawn_editor(&EditorCmd::from_env(&app.env), &target.path)?;
    record_access(app, ui, &cache, &target);
    Ok(ExitCode::SUCCESS)
}

/// `clone <query>`: clones if needed and prints the path.
///
/// # Errors
///
/// Resolution, clone, or cache failures.
pub fn clone(app: &App, ui: &mut Ui, args: &ResolveArgs) -> Result<ExitCode, Error> {
    let (cache, target) = prepare(app, ui, args, Local::Clone)?;
    record_access(app, ui, &cache, &target);
    ui.stdout_line(&target.path.display().to_string())?;
    Ok(ExitCode::SUCCESS)
}

/// `browse <query>`: opens the repo's web page; never clones.
///
/// # Errors
///
/// Resolution, opener, or cache failures.
///
/// # Panics
///
/// Only on a broken [`Target`] invariant (see [`Target::repo`]).
pub fn browse(app: &App, ui: &mut Ui, args: &ResolveArgs) -> Result<ExitCode, Error> {
    let (cache, target) = prepare(app, ui, args, Local::NotNeeded)?;
    ui.flush()?;
    platform::open_url(&target.repo(&cache.repos).html_url)?;
    record_access(app, ui, &cache, &target);
    Ok(ExitCode::SUCCESS)
}

/// `open <query>`: clones if needed and opens the directory.
///
/// # Errors
///
/// Resolution, clone, opener, or cache failures.
pub fn open(app: &App, ui: &mut Ui, args: &ResolveArgs) -> Result<ExitCode, Error> {
    let (cache, target) = prepare(app, ui, args, Local::Clone)?;
    ui.flush()?;
    platform::open_dir(&target.path)?;
    record_access(app, ui, &cache, &target);
    Ok(ExitCode::SUCCESS)
}

/// Fails plain `cd`/`on` with a hint for the detected shell, before any
/// cache or resolution work.
fn require_wrapper(app: &App, via: Invocation) -> Result<(), ShellError> {
    match via {
        Invocation::Wrapped => Ok(()),
        Invocation::Direct => Err(ShellError::NotActive {
            shell: shell::detect(&app.env),
        }),
    }
}

/// Loads the cache (refreshing it when needed) and resolves `query`.
/// Nothing is cloned or recorded yet.
fn resolve(app: &App, ui: &mut Ui, query: &str, pick: bool) -> Result<(Cache, Target), Error> {
    let cache = load_or_refresh(app, ui, Freshness::AutoUpdate)?;
    let target = resolve_target(app, ui, &cache, query, pick)?;
    Ok((cache, target))
}

/// [`resolve`], then clones the result when `local` asks for it. Nothing is
/// recorded yet.
fn prepare(
    app: &App,
    ui: &mut Ui,
    args: &ResolveArgs,
    local: Local,
) -> Result<(Cache, Target), Error> {
    let (cache, target) = resolve(app, ui, &args.query, args.pick)?;
    if local == Local::Clone {
        ensure_cloned(app, ui, &cache, &target)?;
    }
    Ok((cache, target))
}
