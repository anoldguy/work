//! Running child processes, and reducing their stderr to one line.
//!
//! Callers build the `Command`; this module never formats a `Command` with
//! `{:?}` (its argv or environment may hold secrets).

use std::io;
use std::process::{Command, ExitStatus, Output};

/// Runs `cmd` with stdin closed and stdout/stderr captured, and waits.
///
/// # Errors
///
/// The spawn error; `io::ErrorKind::NotFound` means the program is not on
/// `PATH`. A non-zero exit is not an error here; check `Output::status`.
pub fn run_capture(cmd: &mut Command) -> io::Result<Output> {
    let _ = cmd;
    unimplemented!("W1.5")
}

/// Runs `cmd` with stdio inherited (interactive children such as
/// `gh auth login` or the editor), and waits.
///
/// # Errors
///
/// The spawn error; `io::ErrorKind::NotFound` means the program is not on
/// `PATH`.
pub fn run_inherit(cmd: &mut Command) -> io::Result<ExitStatus> {
    let _ = cmd;
    unimplemented!("W1.5")
}

/// Reduces a child's stderr to one line: lossy UTF-8, the last non-empty
/// line (git puts the real error last), with `fatal: `/`error: ` prefixes
/// and control characters removed. Redact the result before showing it.
#[must_use]
pub fn one_line(stderr: &[u8]) -> String {
    let _ = stderr;
    unimplemented!("W1.5")
}
