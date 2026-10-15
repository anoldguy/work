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
pub struct Interactive<'ui>(&'ui mut Ui);

impl<'ui> Interactive<'ui> {
    /// Takes the terminal from `ui`, flushing its buffered stdout first so
    /// earlier output lands before the child's.
    ///
    /// # Errors
    ///
    /// The flush failure ([`UiError::BrokenPipe`] or [`UiError::Io`]).
    pub fn new(ui: &'ui mut Ui) -> Result<Self, UiError> {
        ui.flush()?;
        Ok(Self(ui))
    }

    /// Runs `cmd` with stdio inherited and any live progress suspended, and
    /// waits.
    ///
    /// # Errors
    ///
    /// The spawn error; `io::ErrorKind::NotFound` means the program is not
    /// on `PATH`.
    pub fn run(&mut self, cmd: &mut Command) -> io::Result<ExitStatus> {
        cmd.stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        self.0.suspend_progress(|| cmd.status())
    }
}

/// A child's stderr reduced to one line. Each line (lossy UTF-8) keeps only
/// its last `\r`-separated state, so progress redraws collapse, and is made
/// safe by [`text::one_line`]. The last `fatal: `/`error: ` line wins, with
/// that prefix removed (git puts the real error last); without one, every
/// line that is not a `hint:` is joined with spaces (gh splits one message
/// over several lines).
#[must_use]
pub fn stderr_summary(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let lines: Vec<String> = text
        .split(['\n', '\u{2028}', '\u{2029}'])
        .filter_map(|line| {
            line.split('\r')
                .map(text::one_line)
                .rfind(|state| !state.is_empty())
        })
        .collect();
    let error = lines.iter().rev().find_map(|line| {
        line.strip_prefix("fatal: ")
            .or_else(|| line.strip_prefix("error: "))
    });
    if let Some(error) = error {
        return error.trim().to_owned();
    }
    let told: Vec<&str> = lines
        .iter()
        .map(String::as_str)
        .filter(|line| !line.starts_with("hint:"))
        .collect();
    if told.is_empty() {
        lines.join(" ")
    } else {
        told.join(" ")
    }
}
