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

/// Reduces a child's stderr to one line: lossy UTF-8, the last non-empty
/// line (git puts the real error last), with `fatal: `/`error: ` prefixes
/// and control characters removed.
#[must_use]
pub fn one_line(stderr: &[u8]) -> String {
    let text = strip_escapes(&String::from_utf8_lossy(stderr));
    // `\r` also separates lines so progress redraws yield their final state,
    // and so do the Unicode line and paragraph separators.
    let lines: Vec<String> = text
        .split(['\n', '\r', '\u{2028}', '\u{2029}'])
        .map(clean)
        .filter(|l| !l.is_empty())
        .collect();
    let is_err = |l: &&String| l.starts_with("fatal: ") || l.starts_with("error: ");
    let line = lines
        .iter()
        .rfind(is_err)
        .or_else(|| lines.iter().rfind(|l| !l.starts_with("hint:")))
        .or_else(|| lines.last())
        .cloned()
        .unwrap_or_default();
    line.strip_prefix("fatal: ")
        .or_else(|| line.strip_prefix("error: "))
        .unwrap_or(&line)
        .trim()
        .to_owned()
}

/// Removes ANSI escape sequences: CSI (`ESC [ ... final`), OSC
/// (`ESC ] ... BEL|ST`) and two-byte escapes.
fn strip_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for n in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&n) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(n) = chars.next() {
                    if n == '\x07' || n == '\u{9c}' {
                        break;
                    }
                    if n == '\x1b' {
                        if chars.peek() == Some(&'\\') {
                            chars.next();
                        }
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Maps tabs to spaces, drops control and invisible format characters
/// ([`text::is_unsafe`]), and trims.
fn clean(line: &str) -> String {
    line.chars()
        .filter_map(|c| match c {
            '\t' => Some(' '),
            c if text::is_unsafe(c) => None,
            c => Some(c),
        })
        .collect::<String>()
        .trim()
        .to_owned()
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
    fn one_line_takes_last_nonempty_line_and_strips_prefix() {
        assert_eq!(
            one_line(b"hint: x\nfatal: not a repo\n\n  \n"),
            "not a repo"
        );
        assert_eq!(one_line(b"error: boom"), "boom");
    }

    #[test]
    fn one_line_removes_control_chars_and_handles_cr() {
        assert_eq!(one_line(b"a\rprogress 50%\rdone\x1b[0m\x07"), "done");
        assert_eq!(one_line(b"x\ty\0z\n"), "x yz");
    }

    #[test]
    fn one_line_prefers_error_over_trailing_hint() {
        assert_eq!(
            one_line(b"error: red\nhint: use --foo to override\n"),
            "red"
        );
        assert_eq!(one_line(b"warning: w\nhint: h\n"), "warning: w");
        assert_eq!(one_line(b"hint: only\n"), "hint: only");
    }

    #[test]
    fn one_line_strips_whole_ansi_sequences() {
        assert_eq!(one_line(b"\x1b[1;31mboom\x1b[0m"), "boom");
        assert_eq!(one_line(b"\x1b]0;title\x07ok"), "ok");
        assert_eq!(
            one_line(b"\x1b]8;;http://x\x1b\\link\x1b]8;;\x1b\\"),
            "link"
        );
    }

    #[test]
    fn one_line_drops_bidi_overrides() {
        assert_eq!(one_line("a\u{202e}b\u{200b}c".as_bytes()), "abc");
        for c in [
            '\u{061c}', '\u{200e}', '\u{202a}', '\u{2066}', '\u{2069}', '\u{206f}',
        ] {
            assert_eq!(one_line(format!("x{c}y").as_bytes()), "xy", "{c:?}");
        }
    }

    #[test]
    fn one_line_treats_line_and_paragraph_separators_as_line_breaks() {
        assert_eq!(one_line("first\u{2028}fatal: last".as_bytes()), "last");
        assert_eq!(one_line("a\u{2029}b".as_bytes()), "b");
        assert_eq!(one_line("x\u{2028}\u{2029}".as_bytes()), "x");
        let line = one_line("fatal: a\u{2028}b\u{2029}c".as_bytes());
        assert!(!line.contains(['\u{2028}', '\u{2029}']), "{line:?}");
    }

    #[test]
    fn one_line_handles_empty_and_invalid_utf8() {
        assert_eq!(one_line(b""), "");
        assert_eq!(one_line(b"\n\r\n"), "");
        assert_eq!(one_line(b"bad \xff byte"), "bad \u{fffd} byte");
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
