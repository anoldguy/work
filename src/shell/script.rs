//! The single-line scripts printed by `sh-*` commands (pure).

use std::path::Path;

use crate::shell::ShellError;

/// `cd '<path>'`.
///
/// # Errors
///
/// [`ShellError::NewlineInPath`] or [`ShellError::NonUtf8Path`].
pub fn cd_script(path: &Path) -> Result<String, ShellError> {
    let _ = path;
    unimplemented!("W1.6")
}

/// `cd '<path>' && <editor> '<path>'`; `editor` (the caller passes
/// `EditorCmd::as_str`) is inserted verbatim as a command fragment.
///
/// # Errors
///
/// [`ShellError::NewlineInPath`] or [`ShellError::NonUtf8Path`].
pub fn on_script(path: &Path, editor: &str) -> Result<String, ShellError> {
    let _ = (path, editor);
    unimplemented!("W1.6")
}
