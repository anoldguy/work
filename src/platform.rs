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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{ScriptedInteraction, SharedBuf};
    use crate::ui::Ui;
    use std::assert_matches;

    fn args(cmd: &Command) -> Vec<String> {
        cmd.get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn editor_chain_prefers_work_editor_then_editor_then_vi() {
        let mut env = EnvSnapshot::default();
        assert_eq!(EditorCmd::from_env(&env).as_str(), "vi");
        env.editor = Some("nano".into());
        assert_eq!(EditorCmd::from_env(&env).as_str(), "nano");
        env.work_editor = Some("code -w".into());
        assert_eq!(EditorCmd::from_env(&env).as_str(), "code -w");
    }

    #[test]
    fn blank_work_editor_falls_through_to_editor() {
        let env = EnvSnapshot {
            work_editor: Some("  ".into()),
            editor: Some("nano".into()),
            ..EnvSnapshot::default()
        };
        assert_eq!(EditorCmd::from_env(&env).as_str(), "nano");
    }

    #[test]
    fn editor_command_passes_path_as_argument() {
        let ed = EditorCmd("code -w".into());
        let cmd = ed.command(Path::new("/tmp/a b; rm -rf $HOME"));
        assert_eq!(cmd.get_program(), "/bin/sh");
        assert_eq!(
            args(&cmd),
            ["-c", "code -w \"$@\"", "sh", "/tmp/a b; rm -rf $HOME"]
        );
    }

    #[test]
    fn leading_dash_paths_become_relative() {
        assert_eq!(dash_safe(Path::new("-x")), Path::new("./-x"));
        assert_eq!(dash_safe(Path::new("/a")), Path::new("/a"));
    }

    #[test]
    fn opener_uses_platform_program() {
        let cmd = opener_command("https://example.com".into());
        let expected = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        assert_eq!(cmd.get_program(), expected);
        assert_eq!(args(&cmd), ["https://example.com"]);
    }

    #[test]
    fn spawn_editor_reports_exit_status_with_harmless_binaries() {
        let path = Path::new("/dev/null");
        let mut ui = Ui::for_test(
            Box::new(SharedBuf::new()),
            Box::new(SharedBuf::new()),
            Box::new(ScriptedInteraction::new(false, vec![])),
        );
        let mut term = Interactive::new(&mut ui).expect("flush");
        spawn_editor(&mut term, &EditorCmd("true".into()), path).unwrap();
        assert_matches!(
            spawn_editor(&mut term, &EditorCmd("false".into()), path),
            Err(PlatformError::Failed { .. })
        );
    }
}
