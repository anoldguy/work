//! Building child processes, and reducing their stderr to one line.
//!
//! This module never formats a `Command` with `{:?}` (its argv or
//! environment may hold secrets).

use std::ffi::OsStr;
use std::io;
use std::process::{Command, ExitStatus, Stdio};

use crate::text;
use crate::ui::{Ui, UiError};

/// The only constructor for a child process. The child is captured: stdin
/// is null and stdout and stderr are piped. It gets the terminal only
/// through [`Interactive::run`].
#[expect(
    clippy::disallowed_methods,
    reason = "the one place a child process is built"
)]
#[must_use]
pub fn command(program: impl AsRef<OsStr>) -> Command {
    let mut cmd = Command::new(program);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

/// The terminal, lent by the [`Ui`] to interactive children (the editor,
/// `gh auth`). Holding it borrows the `Ui`, so nothing else can write
/// meanwhile.
#[derive(Debug)]
pub struct Interactive<'ui> {
    _ui: &'ui mut Ui,
}

impl<'ui> Interactive<'ui> {
    /// Takes the terminal from `ui`, flushing its buffered stdout first so
    /// earlier output lands before the child's.
    ///
    /// # Errors
    ///
    /// The flush failure ([`UiError::BrokenPipe`] or [`UiError::Io`]).
    pub fn new(ui: &'ui mut Ui) -> Result<Self, UiError> {
        ui.flush()?;
        Ok(Self { _ui: ui })
    }

    /// Runs `cmd` with stdio inherited and waits.
    ///
    /// # Errors
    ///
    /// The spawn error; `io::ErrorKind::NotFound` means the program is not
    /// on `PATH`.
    #[expect(
        clippy::disallowed_methods,
        reason = "the only child that gets the terminal, while holding the Ui"
    )]
    pub fn run(&mut self, cmd: &mut Command) -> io::Result<ExitStatus> {
        cmd.stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        cmd.status()
    }
}

/// What [`stderr_summary`] keeps when stderr has no `fatal:`/`error:` line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fallback {
    /// The last line (git: earlier lines are progress and `remote:` noise).
    LastLine,
    /// Every line, joined (gh splits one message over several lines).
    AllLines,
}

/// A child's stderr reduced to one line. Each line (lossy UTF-8) keeps only
/// its last `\r`-separated state, so progress redraws collapse, is made safe
/// by [`text::one_line`], and `hint:` lines are dropped. From the first
/// `fatal: `/`error: ` line on, every line is kept with that prefix removed,
/// so a message git spreads over several lines stays whole; without such a
/// line, `fallback` decides.
#[must_use]
pub fn stderr_summary(stderr: &[u8], fallback: Fallback) -> String {
    let text = String::from_utf8_lossy(stderr);
    let lines: Vec<String> = text
        .split(['\n', '\u{2028}', '\u{2029}'])
        .filter_map(|line| {
            line.split('\r')
                .map(text::one_line)
                .rfind(|state| !state.is_empty())
        })
        .collect();
    let mut told: Vec<&str> = lines
        .iter()
        .map(String::as_str)
        .filter(|line| !line.starts_with("hint:"))
        .collect();
    if told.is_empty() {
        told = lines.iter().map(String::as_str).collect();
    }
    if let Some(first) = told.iter().position(|line| error_text(line).is_some()) {
        return told[first..]
            .iter()
            .map(|line| error_text(line).unwrap_or(line))
            .collect::<Vec<_>>()
            .join(" ");
    }
    match fallback {
        Fallback::LastLine => told.last().copied().unwrap_or_default().to_owned(),
        Fallback::AllLines => told.join(" "),
    }
}

/// `line` without its `fatal: `/`error: ` prefix, if it has one.
fn error_text(line: &str) -> Option<&str> {
    line.strip_prefix("fatal: ")
        .or_else(|| line.strip_prefix("error: "))
        .map(str::trim)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{ScriptedInteraction, SharedBuf};
    use std::assert_matches;

    fn sh(script: &str) -> Command {
        let mut c = command("sh");
        c.args(["-c", script]);
        c
    }

    fn interactive(cmd: &mut Command) -> io::Result<ExitStatus> {
        let mut ui = Ui::for_test(
            Box::new(SharedBuf::new()),
            Box::new(SharedBuf::new()),
            Box::new(ScriptedInteraction::new(false, vec![])),
        );
        Interactive::new(&mut ui).expect("flush").run(cmd)
    }

    #[test]
    fn stderr_summary_keeps_the_error_message_from_its_first_line_on() {
        use Fallback::LastLine;
        assert_eq!(
            stderr_summary(
                b"Cloning into 'x'...\nremote: nope\nfatal: not found\n",
                LastLine
            ),
            "not found"
        );
        assert_eq!(
            stderr_summary(
                b"error: Your local changes would be overwritten by merge:\n\ta.txt\nPlease commit your changes or stash them before you merge.\nAborting\n",
                LastLine
            ),
            "Your local changes would be overwritten by merge: a.txt Please commit your changes or stash them before you merge. Aborting"
        );
        assert_eq!(
            stderr_summary(
                b"error: cannot pull with rebase: You have unstaged changes.\nerror: Please commit or stash them.\n",
                LastLine
            ),
            "cannot pull with rebase: You have unstaged changes. Please commit or stash them."
        );
        assert_eq!(
            stderr_summary(b"error: red\nhint: use --foo to override\n", LastLine),
            "red"
        );
    }

    #[test]
    fn stderr_summary_without_an_error_line_follows_the_fallback() {
        let git = b"Cloning into 'p'...\nremote: Counting objects: 100%\nReceiving objects: 100% (8/8), done.\n";
        assert_eq!(
            stderr_summary(git, Fallback::LastLine),
            "Receiving objects: 100% (8/8), done."
        );
        assert_eq!(
            stderr_summary(b"not logged in\nrun gh auth login\n", Fallback::AllLines),
            "not logged in run gh auth login"
        );
        assert_eq!(
            stderr_summary(b"hint: only\n", Fallback::AllLines),
            "hint: only"
        );
    }

    #[test]
    fn stderr_summary_keeps_the_final_progress_state_and_no_controls() {
        let sum = |b: &[u8]| stderr_summary(b, Fallback::AllLines);
        assert_eq!(sum(b"a\rprogress 50%\rdone\x1b[0m\x07\r"), "done");
        assert_eq!(sum(b"\x1b[1;31mboom\x1b[0m"), "boom");
        assert_eq!(sum("a\u{202e}b\u{200b}c".as_bytes()), "abc");
        assert_eq!(sum(b""), "");
        assert_eq!(sum(b"\n\r\n"), "");
        assert_eq!(sum(b"bad \xff byte"), "bad \u{fffd} byte");
    }

    #[test]
    fn capture_returns_output_and_status() {
        let out = sh("echo out; echo err >&2; exit 3").output().unwrap();
        assert_eq!(out.stdout, b"out\n");
        assert_eq!(out.stderr, b"err\n");
        assert_eq!(out.status.code(), Some(3));
    }

    #[test]
    fn capture_closes_stdin() {
        let out = sh("cat; echo done").output().unwrap();
        assert_eq!(out.stdout, b"done\n");
    }

    #[test]
    fn interactive_reports_exit_status() {
        assert!(interactive(&mut command("true")).unwrap().success());
        assert!(!interactive(&mut command("false")).unwrap().success());
    }

    #[test]
    fn missing_binary_is_not_found() {
        let err = command("work-no-such-binary-w15").output().unwrap_err();
        assert_matches!(err.kind(), io::ErrorKind::NotFound);
        let err = interactive(&mut command("work-no-such-binary-w15")).unwrap_err();
        assert_matches!(err.kind(), io::ErrorKind::NotFound);
    }
}
