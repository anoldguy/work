//! `cd`, `on`, `edit`, `clone`, `browse`, `open`, and their `sh-*` forms.
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
use crate::error::Error;
use crate::platform::{self, EditorCmd};
use crate::shell::{self, ShellError};

/// How `cd`/`on` were invoked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Invocation {
    /// `sh-cd`/`sh-on` from the wrapper: print a script on stdout.
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

/// `cd [query]`: prints `cd '<path>'` (the work directory, created if
/// missing, when there is no query).
///
/// # Errors
///
/// Not wrapped, resolution, clone, or cache failures.
pub fn cd(app: &mut App, args: &CdArgs, via: Invocation) -> Result<ExitCode, Error> {
    require_wrapper(app, via)?;
    let Some(query) = &args.query else {
        let dir = app.config.work_dir();
        // Check the path can be printed before creating anything.
        let script = shell::cd_script(dir)?;
        fs::create_dir_all(dir).map_err(|source| Error::CreateDir {
            path: dir.to_path_buf(),
            source,
        })?;
        app.ui.stdout_line(&script)?;
        return Ok(ExitCode::SUCCESS);
    };
    let (mut cache, target) = resolve(app, query, args.pick)?;
    // Check the path can be printed before cloning anything.
    let script = shell::cd_script(&target.path)?;
    ensure_cloned(app, &cache, &target)?;
    record_access(app, &mut cache, &target)?;
    app.ui.stdout_line(&script)?;
    Ok(ExitCode::SUCCESS)
}

/// `on <query>`: prints `cd '<path>' && <editor> '<path>'`.
///
/// # Errors
///
/// Not wrapped, resolution, clone, or cache failures.
pub fn on(app: &mut App, args: &ResolveArgs, via: Invocation) -> Result<ExitCode, Error> {
    require_wrapper(app, via)?;
    let editor = script_editor(app)?;
    let (mut cache, target) = resolve(app, &args.query, args.pick)?;
    // Check the path can be printed before cloning anything.
    let script = shell::on_script(&target.path, editor.as_str())?;
    ensure_cloned(app, &cache, &target)?;
    record_access(app, &mut cache, &target)?;
    app.ui.stdout_line(&script)?;
    Ok(ExitCode::SUCCESS)
}

/// `edit <query>`: clones if needed and runs the editor.
///
/// # Errors
///
/// Resolution, clone, editor, or cache failures.
pub fn edit(app: &mut App, args: &ResolveArgs) -> Result<ExitCode, Error> {
    let (mut cache, target) = prepare(app, args, Local::Clone)?;
    // The editor inherits stdout: pending output must land first.
    app.ui.flush()?;
    platform::spawn_editor(&EditorCmd::from_env(&app.env), &target.path)?;
    record_access(app, &mut cache, &target)?;
    Ok(ExitCode::SUCCESS)
}

/// `clone <query>`: clones if needed and prints the path.
///
/// # Errors
///
/// Resolution, clone, or cache failures.
pub fn clone(app: &mut App, args: &ResolveArgs) -> Result<ExitCode, Error> {
    let (mut cache, target) = prepare(app, args, Local::Clone)?;
    record_access(app, &mut cache, &target)?;
    app.ui.stdout_line(&target.path.display().to_string())?;
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
pub fn browse(app: &mut App, args: &ResolveArgs) -> Result<ExitCode, Error> {
    let (mut cache, target) = prepare(app, args, Local::NotNeeded)?;
    app.ui.flush()?;
    platform::open_url(&target.repo(&cache.repos).html_url)?;
    record_access(app, &mut cache, &target)?;
    Ok(ExitCode::SUCCESS)
}

/// `open <query>`: clones if needed and opens the directory.
///
/// # Errors
///
/// Resolution, clone, opener, or cache failures.
pub fn open(app: &mut App, args: &ResolveArgs) -> Result<ExitCode, Error> {
    let (mut cache, target) = prepare(app, args, Local::Clone)?;
    app.ui.flush()?;
    platform::open_dir(&target.path)?;
    record_access(app, &mut cache, &target)?;
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

/// The editor for `sh-on`'s script. It is inserted verbatim as a command
/// fragment, so a control character (a newline above all) would split the
/// one-line script and run the rest as a separate command: reject it.
fn script_editor(app: &App) -> Result<EditorCmd, ShellError> {
    let editor = EditorCmd::from_env(&app.env);
    if editor.as_str().chars().any(char::is_control) {
        return Err(ShellError::ControlCharInEditor {
            given: editor.as_str().to_owned(),
        });
    }
    Ok(editor)
}

/// Loads the cache (refreshing it when needed) and resolves `query`.
/// Nothing is cloned or recorded yet.
fn resolve(app: &mut App, query: &str, pick: bool) -> Result<(Cache, Target), Error> {
    let mut cache = load_or_refresh(app, Freshness::AutoUpdate)?;
    let target = resolve_target(app, &mut cache, query, pick)?;
    Ok((cache, target))
}

/// [`resolve`], then clones the result when `local` asks for it. Nothing is
/// recorded yet.
fn prepare(app: &mut App, args: &ResolveArgs, local: Local) -> Result<(Cache, Target), Error> {
    let (cache, target) = resolve(app, &args.query, args.pick)?;
    if local == Local::Clone {
        ensure_cloned(app, &cache, &target)?;
    }
    Ok((cache, target))
}
