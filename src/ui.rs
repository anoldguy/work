//! The terminal owner.
//!
//! [`Ui`] is created once, in `lib::run`, and is the only thing that writes
//! to stdout or stderr. stdout carries only command output (and `sh-*`
//! scripts); progress, pickers, prompts and warnings go to stderr. Methods
//! that draw take `&mut self`, so the borrow checker keeps a picker, a
//! progress display and a stdout write from interleaving.
//!
//! `impl Ui` is split across files: `ui/progress.rs` (`with_progress`),
//! `ui/terminal.rs` (`terminal`) and `ui/interact.rs` (`interact`).

pub mod interact;
pub mod picker;
pub mod progress;
pub mod prompt;
pub mod splash;
pub mod terminal;

pub use interact::TtyInteraction;
pub use terminal::{Screen, TerminalGuard};

use std::fmt;
use std::io::{self, BufWriter, IsTerminal, Write};

use crate::env::EnvSnapshot;

/// How stderr is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// stderr is a terminal: live progress, colour unless `NO_COLOR`.
    Tty,
    /// stderr is not a terminal: no live progress, only final warnings.
    Plain,
}

/// The sole terminal owner.
pub struct Ui {
    mode: Mode,
    color: bool,
    out: BufWriter<Box<dyn Write + Send>>,
    err: Box<dyn Write + Send>,
    deferred: Vec<String>,
    interact: Box<dyn Interaction>,
}

impl Ui {
    /// The real terminal: `Tty` iff stderr is a terminal; colour iff `Tty`
    /// and not `NO_COLOR`; a [`TtyInteraction`].
    #[must_use]
    pub fn new(env: &EnvSnapshot) -> Self {
        // `TERM=dumb` terminals (Emacs shell, some CI ptys) cannot redraw.
        let tty = io::stderr().is_terminal() && env.term.as_deref() != Some("dumb");
        Self {
            mode: if tty { Mode::Tty } else { Mode::Plain },
            color: tty && !env.no_color,
            out: BufWriter::new(Box::new(io::stdout())),
            err: Box::new(io::stderr()),
            deferred: Vec::new(),
            interact: Box::new(TtyInteraction::for_env(env)),
        }
    }

    /// A `Plain`, colourless Ui over the given writers and interaction, for
    /// tests.
    #[must_use]
    pub fn for_test(
        out: Box<dyn Write + Send>,
        err: Box<dyn Write + Send>,
        interact: Box<dyn Interaction>,
    ) -> Self {
        Self::for_test_mode(Mode::Plain, out, err, interact)
    }

    /// Like [`Ui::for_test`] with an explicit `mode`, so the `Tty` redraw
    /// path can be exercised over in-memory writers. Never colourised.
    #[must_use]
    pub fn for_test_mode(
        mode: Mode,
        out: Box<dyn Write + Send>,
        err: Box<dyn Write + Send>,
        interact: Box<dyn Interaction>,
    ) -> Self {
        Self {
            mode,
            color: false,
            out: BufWriter::new(out),
            err,
            deferred: Vec::new(),
            interact,
        }
    }

    /// How stderr is used.
    #[must_use]
    pub const fn mode(&self) -> Mode {
        self.mode
    }

    /// Whether output may use colour.
    #[must_use]
    pub const fn color(&self) -> bool {
        self.color
    }

    /// Writes one line of command output to stdout (buffered).
    ///
    /// # Errors
    ///
    /// [`UiError::BrokenPipe`] when stdout's reader went away (the command
    /// should stop and exit 0 silently), or [`UiError::Io`].
    pub fn stdout_line(&mut self, line: &str) -> Result<(), UiError> {
        writeln!(self.out, "{line}").map_err(UiError::from_stdout)
    }

    /// Writes one informational line to stderr immediately, after flushing
    /// pending stdout so the two streams keep their order (e.g. the zsh
    /// `fpath` hint, or `about`'s static logo).
    ///
    /// # Errors
    ///
    /// [`UiError::Io`].
    pub fn stderr_line(&mut self, line: &str) -> Result<(), UiError> {
        self.flush_for_stderr();
        writeln!(self.err, "{line}")
            .and_then(|()| self.err.flush())
            .map_err(|source| UiError::Io { source })
    }

    /// Prints a one-line warning to stderr at once, after flushing pending
    /// stdout. Never fails; a broken stderr loses the warning. (During
    /// `with_progress` the `&mut Ui` borrow is held, so warnings from workers
    /// arrive as events and are flushed through `deferred`.)
    pub fn warn(&mut self, message: &str) {
        debug_assert!(
            !message.contains('\n'),
            "warnings are one line: {message:?}"
        );
        self.flush_for_stderr();
        let _ = writeln!(self.err, "warning: {message}").and_then(|()| self.err.flush());
    }

    /// Best-effort stdout flush before writing to stderr; a real failure
    /// resurfaces from the next [`Ui::flush`].
    fn flush_for_stderr(&mut self) {
        let _ = self.out.flush();
    }

    /// Flushes buffered stdout.
    ///
    /// # Errors
    ///
    /// [`UiError::BrokenPipe`] or [`UiError::Io`].
    pub fn flush(&mut self) -> Result<(), UiError> {
        self.out.flush().map_err(UiError::from_stdout)
    }
}

impl fmt::Debug for Ui {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ui")
            .field("mode", &self.mode)
            .field("color", &self.color)
            .field("deferred", &self.deferred.len())
            .finish_non_exhaustive()
    }
}

impl UiError {
    /// Maps a stdout write failure: a closed reader is [`UiError::BrokenPipe`].
    fn from_stdout(source: io::Error) -> Self {
        if source.kind() == io::ErrorKind::BrokenPipe {
            Self::BrokenPipe
        } else {
            Self::Io { source }
        }
    }
}

/// Pickers and prompts, all drawn on stderr. Behind a trait so tests can
/// script answers.
///
/// Every method that waits for input returns [`UiError::Interrupted`] on
/// Ctrl-C (exit 130).
pub trait Interaction {
    /// True when stdin and stderr are both terminals.
    fn is_interactive(&self) -> bool;

    /// Type-to-filter single choice. `Ok(None)` when cancelled with Esc.
    ///
    /// # Errors
    ///
    /// [`UiError::NotInteractive`], [`UiError::Interrupted`], or
    /// [`UiError::Io`].
    fn pick_one(&mut self, prompt: &str, items: &[&str]) -> Result<Option<usize>, UiError>;

    /// Type-to-filter multi choice with `selected` preselected (same length
    /// as `items`); Tab toggles. `Ok(None)` when cancelled.
    ///
    /// # Errors
    ///
    /// [`UiError::NotInteractive`], [`UiError::Interrupted`], or
    /// [`UiError::Io`].
    fn pick_many(
        &mut self,
        prompt: &str,
        items: &[&str],
        selected: &[bool],
    ) -> Result<Option<Vec<bool>>, UiError>;

    /// A line of text with `default` offered. `Ok(None)` when cancelled.
    ///
    /// # Errors
    ///
    /// [`UiError::NotInteractive`], [`UiError::Interrupted`], or
    /// [`UiError::Io`].
    fn input(&mut self, prompt: &str, default: &str) -> Result<Option<String>, UiError>;

    /// A yes/no question; Enter takes `default`.
    ///
    /// # Errors
    ///
    /// [`UiError::NotInteractive`], [`UiError::Interrupted`], or
    /// [`UiError::Io`].
    fn confirm(&mut self, prompt: &str, default: bool) -> Result<bool, UiError>;
}

/// A terminal failure.
#[derive(Debug, thiserror::Error)]
pub enum UiError {
    /// stdout's reader went away (`work list | head`). Exit 0, print nothing.
    #[error("broken pipe")]
    BrokenPipe,
    /// Ctrl-C in a picker or prompt. Exit 130.
    #[error("interrupted")]
    Interrupted,
    /// A command or flag needs a terminal and there is none.
    #[error("{what} requires an interactive terminal")]
    NotInteractive {
        /// What needed it, e.g. `--pick`, `setup`, `favorite`.
        what: &'static str,
    },
    /// Any other terminal I/O failure.
    #[error("terminal error: {source}")]
    Io {
        /// The I/O error.
        source: io::Error,
    },
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::io::{self, Write};

    use super::{Mode, Ui, UiError};
    use crate::testing::{ScriptedInteraction, SharedBuf};

    /// A writer that fails every write with `kind`.
    struct Failing(io::ErrorKind);

    impl Write for Failing {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(self.0))
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::from(self.0))
        }
    }

    fn ui(out: Box<dyn Write + Send>, err: Box<dyn Write + Send>) -> Ui {
        Ui::for_test(out, err, Box::new(ScriptedInteraction::new(false, vec![])))
    }

    #[test]
    fn stdout_is_buffered_until_flush_and_stderr_is_immediate() {
        let (out, err) = (SharedBuf::new(), SharedBuf::new());
        let mut ui = ui(Box::new(out.clone()), Box::new(err.clone()));
        assert_eq!(ui.mode(), Mode::Plain);
        assert!(!ui.color());
        ui.stdout_line("one").expect("write");
        assert_eq!(out.contents(), "");
        ui.flush().expect("flush");
        assert_eq!(out.contents(), "one\n");
        ui.stderr_line("hint").expect("write");
        ui.warn("careful");
        assert_eq!(err.contents(), "hint\nwarning: careful\n");
    }

    #[test]
    fn a_closed_stdout_reader_is_a_broken_pipe() {
        let mut ui = ui(
            Box::new(Failing(io::ErrorKind::BrokenPipe)),
            Box::new(SharedBuf::new()),
        );
        ui.stdout_line("x").expect("buffered");
        assert_matches!(ui.flush(), Err(UiError::BrokenPipe));
    }

    #[test]
    fn other_stdout_failures_are_io_errors() {
        let mut ui = ui(
            Box::new(Failing(io::ErrorKind::PermissionDenied)),
            Box::new(SharedBuf::new()),
        );
        ui.stdout_line("x").expect("buffered");
        assert_matches!(ui.flush(), Err(UiError::Io { .. }));
    }

    #[test]
    fn stderr_output_waits_for_pending_stdout() {
        let (out, err) = (SharedBuf::new(), SharedBuf::new());
        let mut ui = ui(Box::new(out.clone()), Box::new(err));
        ui.stdout_line("first").expect("write");
        ui.warn("second");
        assert_eq!(out.contents(), "first\n");
    }

    #[test]
    fn tty_mode_can_be_built_for_tests() {
        let (out, err) = (SharedBuf::new(), SharedBuf::new());
        let ui = Ui::for_test_mode(
            Mode::Tty,
            Box::new(out),
            Box::new(err),
            Box::new(ScriptedInteraction::new(false, vec![])),
        );
        assert_eq!(ui.mode(), Mode::Tty);
        assert!(!ui.color());
    }

    #[test]
    fn warn_never_fails_on_a_broken_stderr() {
        let mut ui = ui(
            Box::new(SharedBuf::new()),
            Box::new(Failing(io::ErrorKind::BrokenPipe)),
        );
        ui.warn("lost");
        assert_matches!(ui.stderr_line("x"), Err(UiError::Io { .. }));
    }
}
