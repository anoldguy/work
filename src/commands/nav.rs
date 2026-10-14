//! `cd`, `on`, `edit`, `clone`, `browse`, `open`, and their `sh-*` forms.

use std::process::ExitCode;

use crate::app::App;
use crate::cli::{CdArgs, ResolveArgs};
use crate::error::Error;

/// How `cd`/`on` were invoked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Invocation {
    /// `sh-cd`/`sh-on` from the wrapper: print a script on stdout.
    Wrapped,
    /// Plain `cd`/`on`: fail with `ShellError::NotActive` without resolving.
    Direct,
}

/// `cd [query]`: prints `cd '<path>'` (the work directory, created if
/// missing, when there is no query).
///
/// # Errors
///
/// Not wrapped, resolution, clone, or cache failures.
pub fn cd(app: &mut App, args: CdArgs, via: Invocation) -> Result<ExitCode, Error> {
    let _ = (app, args, via);
    unimplemented!("W5.1")
}

/// `on <query>`: prints `cd '<path>' && <editor> '<path>'`.
///
/// # Errors
///
/// Not wrapped, resolution, clone, or cache failures.
pub fn on(app: &mut App, args: ResolveArgs, via: Invocation) -> Result<ExitCode, Error> {
    let _ = (app, args, via);
    unimplemented!("W5.1")
}

/// `edit <query>`: clones if needed and runs the editor.
///
/// # Errors
///
/// Resolution, clone, editor, or cache failures.
pub fn edit(app: &mut App, args: ResolveArgs) -> Result<ExitCode, Error> {
    let _ = (app, args);
    unimplemented!("W5.1")
}

/// `clone <query>`: clones if needed and prints the path.
///
/// # Errors
///
/// Resolution, clone, or cache failures.
pub fn clone(app: &mut App, args: ResolveArgs) -> Result<ExitCode, Error> {
    let _ = (app, args);
    unimplemented!("W5.1")
}

/// `browse <query>`: opens the repo's web page; never clones.
///
/// # Errors
///
/// Resolution, opener, or cache failures.
pub fn browse(app: &mut App, args: ResolveArgs) -> Result<ExitCode, Error> {
    let _ = (app, args);
    unimplemented!("W5.1")
}

/// `open <query>`: clones if needed and opens the directory.
///
/// # Errors
///
/// Resolution, clone, opener, or cache failures.
pub fn open(app: &mut App, args: ResolveArgs) -> Result<ExitCode, Error> {
    let _ = (app, args);
    unimplemented!("W5.1")
}
