//! [`TerminalGuard`]: the only way to get raw mode or the alternate screen.

use std::fmt;
use std::io::{self, Write};
use std::time::Duration;

use crossterm::event::KeyEvent;

use crate::ui::{Ui, UiError};

/// Where a guarded session draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// Below the cursor in the normal screen (pickers, prompts).
    Inline,
    /// The alternate screen, leaving scrollback untouched (`about`).
    Alternate,
}

/// RAII over a raw-mode session on stderr: raw mode, hidden cursor, and
/// optionally the alternate screen.
///
/// Restores the terminal on `Drop`, and on panic through a scoped panic hook
/// installed by [`TerminalGuard::acquire`]. Writes go to the borrowed stderr
/// writer through the `Write` impl.
pub struct TerminalGuard<'a> {
    out: &'a mut (dyn Write + Send),
    screen: Screen,
}

impl<'a> TerminalGuard<'a> {
    /// Enters raw mode on `out` (and the alternate screen if asked) and hides
    /// the cursor.
    ///
    /// # Errors
    ///
    /// [`UiError::Io`] when the terminal refuses.
    pub fn acquire(out: &'a mut (dyn Write + Send), screen: Screen) -> Result<Self, UiError> {
        let _ = (out, screen);
        unimplemented!("W1.7")
    }

    /// Waits up to `timeout` for a key press. `Ok(None)` on timeout.
    ///
    /// # Errors
    ///
    /// [`UiError::Io`].
    pub fn read_key(&mut self, timeout: Duration) -> Result<Option<KeyEvent>, UiError> {
        let _ = (self, timeout);
        unimplemented!("W1.7")
    }

    /// The terminal size as `(columns, rows)`.
    ///
    /// # Errors
    ///
    /// [`UiError::Io`].
    pub fn size(&self) -> Result<(u16, u16), UiError> {
        let _ = self;
        unimplemented!("W1.7")
    }
}

impl Write for TerminalGuard<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.out.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

impl Drop for TerminalGuard<'_> {
    fn drop(&mut self) {
        // W1.7: show the cursor, leave the alternate screen (if entered) and
        // raw mode, ignoring errors. Until then `acquire` cannot succeed, so
        // there is nothing to restore.
    }
}

impl fmt::Debug for TerminalGuard<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalGuard")
            .field("screen", &self.screen)
            .finish_non_exhaustive()
    }
}

impl Ui {
    /// A guarded raw-mode session on this Ui's stderr. Holding the guard
    /// borrows the Ui, so nothing else can draw meanwhile.
    ///
    /// # Errors
    ///
    /// [`UiError::NotInteractive`] in `Plain` mode, or [`UiError::Io`].
    pub fn terminal(&mut self, screen: Screen) -> Result<TerminalGuard<'_>, UiError> {
        let _ = (&mut self.err, screen);
        unimplemented!("W1.7")
    }
}
