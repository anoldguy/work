//! The terminal owner.
//!
//! [`Ui`] is the only thing that writes to stdout or stderr. Methods that
//! draw take `&mut self`, so a picker, a progress display and a stdout write
//! cannot interleave.

pub mod interact;
pub mod picker;
pub mod prompt;
pub mod splash;
pub mod terminal;

pub use interact::TtyInteraction;
pub use terminal::{Screen, TerminalGuard};

use std::fmt;
use std::io::{self, BufWriter, IsTerminal, Write};
use std::sync::Arc;

use indicatif::{MultiProgress, ProgressDrawTarget};

use crate::env::EnvSnapshot;
use crate::progress::Tasks;
use terminal::{Backend, Crossterm};

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
    /// Live progress: stderr in `Tty` mode, hidden in `Plain` mode.
    progress: MultiProgress,
    interact: Box<dyn Interaction>,
    backend: Arc<dyn Backend>,
}

impl Ui {
    /// The real terminal: `Tty` iff stderr is a terminal; colour iff `Tty`
    /// and not `NO_COLOR`; a [`TtyInteraction`], interactive iff `Tty` and
    /// stdin is a terminal.
    #[expect(
        clippy::disallowed_methods,
        reason = "the terminal owner holds the process's stdout and stderr"
    )]
    #[must_use]
    pub fn new(env: &EnvSnapshot) -> Self {
        // `TERM=dumb` terminals (Emacs shell, some CI ptys) cannot redraw.
        let tty = io::stderr().is_terminal() && env.term.as_deref() != Some("dumb");
        let backend: Arc<dyn Backend> = Arc::new(Crossterm::new(io::stderr()));
        let interactive = tty && io::stdin().is_terminal();
        Self {
            mode: if tty { Mode::Tty } else { Mode::Plain },
            color: tty && !env.no_color,
            out: BufWriter::new(Box::new(io::stdout())),
            err: Box::new(io::stderr()),
            progress: MultiProgress::with_draw_target(if tty {
                ProgressDrawTarget::stderr()
            } else {
                ProgressDrawTarget::hidden()
            }),
            interact: Box::new(TtyInteraction::new(
                Box::new(io::stderr()),
                interactive,
                Arc::clone(&backend),
            )),
            backend,
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

impl Ui {
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

/// Pickers and prompts, all drawn on stderr.
///
/// Every method that waits for input returns [`UiError::Interrupted`] on
/// Ctrl-C.
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
