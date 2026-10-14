//! Editor resolution, and opening URLs and directories with the platform's
//! opener (`xdg-open` on Linux, `open` on macOS).

use std::io;
use std::path::Path;
use std::process::ExitStatus;

use crate::domain::HtmlUrl;
use crate::env::EnvSnapshot;

/// The editor command: `$WORK_EDITOR`, then `$EDITOR`, then `vi`.
///
/// Used verbatim as a shell command fragment (it may carry arguments, like
/// `code -w`), the same way git treats `$EDITOR`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorCmd(String);

impl EditorCmd {
    /// Applies the editor chain to `env`.
    #[must_use]
    pub fn from_env(env: &EnvSnapshot) -> Self {
        let _ = env;
        unimplemented!("W1.11")
    }

    /// The command fragment.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Runs `sh -c '<editor> "$1"' sh <path>` with stdio inherited, and waits.
///
/// # Errors
///
/// [`PlatformError::Spawn`] or [`PlatformError::Failed`].
pub fn spawn_editor(editor: &EditorCmd, path: &Path) -> Result<(), PlatformError> {
    let _ = (editor, path);
    unimplemented!("W1.11")
}

/// Opens `url` in the default browser, detached.
///
/// # Errors
///
/// [`PlatformError::Spawn`] when the opener cannot be started.
pub fn open_url(url: &HtmlUrl) -> Result<(), PlatformError> {
    let _ = url;
    unimplemented!("W1.11")
}

/// Opens `path` in the file manager, detached.
///
/// # Errors
///
/// [`PlatformError::Spawn`] when the opener cannot be started.
pub fn open_dir(path: &Path) -> Result<(), PlatformError> {
    let _ = path;
    unimplemented!("W1.11")
}

/// A failure starting or running an external program. `program` comes from
/// the environment, so it renders with `{:?}`.
#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    /// The program could not be started.
    #[error("cannot run {program:?}: {source}")]
    Spawn {
        /// The program or editor command.
        program: String,
        /// The spawn error.
        source: io::Error,
    },
    /// The program exited unsuccessfully.
    #[error("{program:?} failed ({status})")]
    Failed {
        /// The program or editor command.
        program: String,
        /// How it exited.
        status: ExitStatus,
    },
}
