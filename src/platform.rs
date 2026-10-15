//! Editor resolution, and opening URLs and directories with the platform's
//! opener (`xdg-open` on Linux, `open` on macOS).

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

use crate::domain::HtmlUrl;
use crate::env::EnvSnapshot;
use crate::proc::{self, Interactive};

/// The shell used to run the editor, by absolute path so a restricted
/// `PATH` cannot hide it.
const SH: &str = "/bin/sh";

/// The URL and directory opener for this platform.
#[cfg(target_os = "macos")]
const OPENER: &str = "open";
/// The URL and directory opener for this platform.
#[cfg(not(target_os = "macos"))]
const OPENER: &str = "xdg-open";

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
        let usable = |e: &&str| !e.trim().is_empty();
        let chosen = env
            .work_editor
            .as_deref()
            .filter(usable)
            .or_else(|| env.editor.as_deref().filter(usable));
        Self(chosen.unwrap_or("vi").to_owned())
    }

    /// Builds `sh -c '<editor> "$@"' sh <path>`, so the path is an argument
    /// and never interpolated into the shell text.
    fn command(&self, path: &Path) -> Command {
        let mut cmd = proc::command(SH);
        cmd.arg("-c")
            .arg(format!("{} \"$@\"", self.0))
            .arg("sh")
            .arg(dash_safe(path));
        cmd
    }

    /// The command fragment.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Runs `sh -c '<editor> "$@"' sh <path>` on `term`, and waits.
///
/// # Errors
///
/// [`PlatformError::Spawn`] or [`PlatformError::Failed`].
pub fn spawn_editor(
    term: &mut Interactive<'_>,
    editor: &EditorCmd,
    path: &Path,
) -> Result<(), PlatformError> {
    let program = editor.as_str();
    let status = term
        .run(&mut editor.command(path))
        .map_err(|source| PlatformError::Spawn {
            program: SH.to_owned(),
            source,
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(PlatformError::Failed {
            program: program.to_owned(),
            status,
        })
    }
}

/// Opens `url` in the default browser, detached. The child is never waited
/// on (it is reaped when `work` exits, which is fine for a short-lived CLI).
///
/// # Errors
///
/// [`PlatformError::Spawn`] when the opener cannot be started.
pub fn open_url(url: &HtmlUrl) -> Result<(), PlatformError> {
    spawn_detached(opener_command(OsString::from(url.as_str())))
}

/// Opens `path` in the file manager, detached.
///
/// # Errors
///
/// [`PlatformError::Spawn`] when the opener cannot be started.
pub fn open_dir(path: &Path) -> Result<(), PlatformError> {
    spawn_detached(opener_command(dash_safe(path).into_os_string()))
}

/// `OPENER <arg>`.
fn opener_command(arg: OsString) -> Command {
    let mut cmd = proc::command(OPENER);
    cmd.arg(arg);
    cmd
}

/// Prefixes a leading-dash relative path with `./` so it is not an option.
fn dash_safe(path: &Path) -> PathBuf {
    if path.to_string_lossy().starts_with('-') {
        Path::new(".").join(path)
    } else {
        path.to_path_buf()
    }
}

/// Starts `cmd` with all stdio closed and does not wait for it.
fn spawn_detached(mut cmd: Command) -> Result<(), PlatformError> {
    cmd.stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|source| PlatformError::Spawn {
            program: OPENER.to_owned(),
            source,
        })
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
