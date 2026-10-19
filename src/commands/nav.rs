//! `cd`, `on`, `edit`, `clone`, `browse`, `open`, their `sh-*` forms, and
//! `sh-editor`.
//!
//! Access is recorded only after the action succeeds, so a failed clone,
//! editor or opener records nothing.

use std::fs;
use std::process::ExitCode;

use crate::app::App;
use crate::cache::Cache;
use crate::cache::refresh::Freshness;
use crate::cli::{CdArgs, ResolveArgs};
use crate::commands::resolve::{
    Reach, Target, ensure_cloned, load_known, record_access, resolve_target,
};
use crate::env::EnvSnapshot;
use crate::error::Error;
use crate::platform::{self, EditorCmd};
use crate::proc::Interactive;
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
    /// Never clone.
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

/// `on <query>`: prints the repo's absolute path for the wrapper to `cd` to
/// and open in the editor.
///
/// # Errors
///
/// Not wrapped, an unprintable path, resolution, clone, or cache failures.
pub fn on(app: &App, ui: &mut Ui, args: &ResolveArgs, via: Invocation) -> Result<ExitCode, Error> {
    require_wrapper(app, via)?;
    wrapped_target(app, ui, &args.query, args.pick)
}

/// `sh-editor`: runs the editor on the current directory with the terminal
/// attached.
///
/// # Errors
///
/// The current directory is unreadable, or the editor fails.
pub fn sh_editor(env: &EnvSnapshot, ui: &mut Ui) -> Result<ExitCode, Error> {
    let dir = std::env::current_dir().map_err(|source| ShellError::CurrentDir { source })?;
    platform::spawn_editor(&mut Interactive::new(ui)?, &EditorCmd::from_env(env), &dir)?;
    Ok(ExitCode::SUCCESS)
}

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
    platform::spawn_editor(
        &mut Interactive::new(ui)?,
        &EditorCmd::from_env(&app.env),
        &target.path,
    )?;
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
/// Only on a broken [`Target`] invariant.
pub fn browse(app: &App, ui: &mut Ui, args: &ResolveArgs) -> Result<ExitCode, Error> {
    let (cache, target) = prepare(app, ui, args, Local::NotNeeded)?;
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
    platform::open_dir(&target.path)?;
    record_access(app, ui, &cache, &target);
    Ok(ExitCode::SUCCESS)
}

/// Runs before any cache or resolution work, so plain `cd`/`on` fail fast.
fn require_wrapper(app: &App, via: Invocation) -> Result<(), ShellError> {
    match via {
        Invocation::Wrapped => Ok(()),
        Invocation::Direct => Err(ShellError::NotActive {
            shell: shell::detect(&app.env),
        }),
    }
}

fn resolve(app: &App, ui: &mut Ui, query: &str, pick: bool) -> Result<(Cache, Target), Error> {
    let mut cache = load_known(app, ui, Freshness::AutoUpdate)?;
    let target = resolve_target(app, ui, &mut cache, query, pick, Reach::Readable)?;
    Ok((cache, target))
}

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
