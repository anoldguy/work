//! The terminal owner.
//!
//! [`Ui`] is the only thing that writes to stdout or stderr. Methods that
//! draw take `&mut self`, so a picker, a progress display and a stdout write
//! cannot interleave.

pub mod interact;
pub mod picker;
pub mod progress;
mod prompt;
pub mod splash;
pub mod terminal;

pub use terminal::{Screen, TerminalGuard};

use std::fmt;
use std::io::{self, BufWriter, IsTerminal, Write};

use indicatif::{MultiProgress, ProgressDrawTarget};

use crate::env::EnvSnapshot;
use progress::Tasks;

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
    interactive: bool,
    out: BufWriter<io::Stdout>,
    err: io::Stderr,
    /// Live progress: stderr in `Tty` mode, hidden in `Plain` mode.
    progress: MultiProgress,
}

impl Ui {
    /// The real terminal: `Tty` iff stderr is a terminal; colour iff `Tty`
    /// and not `NO_COLOR`; interactive iff `Tty` and stdin is a terminal.
    #[expect(
        clippy::disallowed_methods,
        reason = "the terminal owner holds the process's stdout and stderr"
    )]
    #[must_use]
    pub fn new(env: &EnvSnapshot) -> Self {
        // `TERM=dumb` terminals (Emacs shell, some CI ptys) cannot redraw.
        let tty = io::stderr().is_terminal() && env.term.as_deref() != Some("dumb");
        let interactive = tty && io::stdin().is_terminal();
        Self {
            mode: if tty { Mode::Tty } else { Mode::Plain },
            color: tty && !env.no_color,
            interactive,
            out: BufWriter::new(io::stdout()),
            err: io::stderr(),
            progress: MultiProgress::with_draw_target(if tty {
                ProgressDrawTarget::stderr()
            } else {
                ProgressDrawTarget::hidden()
            }),
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

    /// True when stdin and stderr are both terminals.
    #[must_use]
    pub const fn is_interactive(&self) -> bool {
        self.interactive
    }

    /// Writes one line of command output to stdout (buffered).
    ///
    /// # Errors
    ///
    /// [`UiError::BrokenPipe`] when stdout's reader went away, or
    /// [`UiError::Io`].
    pub fn stdout_line(&mut self, line: &str) -> Result<(), UiError> {
        writeln!(self.out, "{line}").map_err(UiError::from_stdout)
    }

    /// Writes one informational line to stderr immediately, after flushing
    /// pending stdout so the two streams keep their order.
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
    /// stdout. Never fails; a broken stderr loses the warning.
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
    pub(crate) fn flush_for_stderr(&mut self) {
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

    /// Runs `f` with live progress on stderr (`Tty` mode; hidden in `Plain`
    /// mode). `title` is shown until [`Tasks::set_total`]. When `f` returns,
    /// the display is cleared, the summary line (e.g. `✓ 12 sources ·
    /// 1,204 repos · 1 failed`) is printed in `Tty` mode, and then the
    /// deferred warnings are printed in both modes.
    ///
    /// # Panics
    ///
    /// Resumes a panic from `f` after printing the deferred warnings; the
    /// last progress frame may be left on screen.
    pub fn with_progress<R>(&mut self, title: &str, f: impl FnOnce(&Tasks) -> R) -> R {
        // Anything already printed must land above the display.
        self.flush_for_stderr();
        let tasks = Tasks::new(self.progress.clone(), title, self.color);
        let ending = Ending {
            ui: self,
            tasks: &tasks,
        };
        let result = f(ending.tasks);
        drop(ending);
        result
    }
}

/// Ends a `with_progress` call on return or unwind: clears the display,
/// prints the summary (`Tty`, not while panicking) and the warnings.
struct Ending<'a> {
    ui: &'a mut Ui,
    tasks: &'a Tasks,
}

impl Drop for Ending<'_> {
    fn drop(&mut self) {
        let (summary, warnings) = self.tasks.end();
        if self.ui.mode == Mode::Tty
            && !std::thread::panicking()
            && let Some(summary) = summary
        {
            let _ = writeln!(self.ui.err, "{summary}").and_then(|()| self.ui.err.flush());
        }
        for message in &warnings {
            self.ui.warn(message);
        }
    }
}

impl fmt::Debug for Ui {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ui")
            .field("mode", &self.mode)
            .field("color", &self.color)
            .field("interactive", &self.interactive)
            .finish_non_exhaustive()
    }
}

impl UiError {
    fn from_stdout(source: io::Error) -> Self {
        if source.kind() == io::ErrorKind::BrokenPipe {
            Self::BrokenPipe
        } else {
            Self::Io { source }
        }
    }
}

/// A terminal failure.
#[derive(Debug, thiserror::Error)]
pub enum UiError {
    /// stdout's reader went away (`work list | head`).
    #[error("broken pipe")]
    BrokenPipe,
    /// Ctrl-C in a picker or prompt.
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
