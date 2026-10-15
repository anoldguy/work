//! Running child processes, and reducing their stderr to one line.
//!
//! Callers build the `Command`; this module never formats a `Command` with
//! `{:?}` (its argv or environment may hold secrets).

use std::io;
use std::process::{Command, ExitStatus, Output, Stdio};

use crate::text;

/// Runs `cmd` with stdin closed and stdout/stderr captured, and waits.
///
/// # Errors
///
/// The spawn error; `io::ErrorKind::NotFound` means the program is not on
/// `PATH`. A non-zero exit is not an error here; check `Output::status`.
pub fn run_capture(cmd: &mut Command) -> io::Result<Output> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
}

/// Runs `cmd` with stdio inherited (interactive children such as
/// `gh auth login` or the editor), and waits.
///
/// Caller obligation: stdout is buffered in `Ui`. Flush `Ui` before calling
/// this, or the child's output can appear ahead of earlier output.
///
/// # Errors
///
/// The spawn error; `io::ErrorKind::NotFound` means the program is not on
/// `PATH`.
pub fn run_inherit(cmd: &mut Command) -> io::Result<ExitStatus> {
    cmd.stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
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
    use std::assert_matches;

    fn sh(script: &str) -> Command {
        let mut c = Command::new("sh");
        c.args(["-c", script]);
        c
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
        let out = run_capture(&mut sh("echo out; echo err >&2; exit 3")).unwrap();
        assert_eq!(out.stdout, b"out\n");
        assert_eq!(out.stderr, b"err\n");
        assert_eq!(out.status.code(), Some(3));
    }

    #[test]
    fn capture_closes_stdin() {
        let out = run_capture(&mut sh("cat; echo done")).unwrap();
        assert_eq!(out.stdout, b"done\n");
    }

    #[test]
    fn child_env_is_exactly_what_caller_passes() {
        let mut c = sh("printf '%s|%s' \"$A\" \"$HOME\"");
        c.env_clear().env("A", "1");
        let out = run_capture(&mut c).unwrap();
        assert_eq!(out.stdout, b"1|");
    }

    #[test]
    fn inherit_reports_exit_status() {
        assert!(run_inherit(&mut Command::new("true")).unwrap().success());
        assert!(!run_inherit(&mut Command::new("false")).unwrap().success());
    }

    #[test]
    fn missing_binary_is_not_found() {
        let err = run_capture(&mut Command::new("work-no-such-binary-w15")).unwrap_err();
        assert_matches!(err.kind(), io::ErrorKind::NotFound);
        let err = run_inherit(&mut Command::new("work-no-such-binary-w15")).unwrap_err();
        assert_matches!(err.kind(), io::ErrorKind::NotFound);
    }
}
