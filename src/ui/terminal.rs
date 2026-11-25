//! [`TerminalGuard`]: the only way to get raw mode or the alternate screen.

use std::fmt;
use std::io::{self, Write};
use std::panic;
use std::sync::{Mutex, Once, PoisonError, TryLockError};
use std::time::{Duration, Instant};

use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event, KeyEvent, KeyEventKind};
use crossterm::queue;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};

use crate::ui::{Mode, Ui, UiError};

/// Where a guarded session draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// Below the cursor in the normal screen.
    Inline,
    /// The alternate screen, leaving scrollback untouched.
    Alternate,
}

/// Set by `acquire`, cleared by `Drop` (or taken by the panic hook).
static ACTIVE: Mutex<Option<Screen>> = Mutex::new(None);

/// Installs (once per process) the panic hook that restores the active
/// guard session, if any, and then chains to the previous hook, so its
/// message is readable.
///
/// A panic on any thread restores the terminal, even from under a guard
/// owned by another thread; that guard's `Drop` then restores again,
/// harmlessly.
fn install_panic_hook() {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let screen = match ACTIVE.try_lock() {
                Ok(mut slot) => slot.take(),
                Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner().take(),
                Err(TryLockError::WouldBlock) => None,
            };
            if let Some(screen) = screen {
                #[expect(
                    clippy::disallowed_methods,
                    reason = "panic hook restores the terminal while Ui may be inaccessible"
                )]
                let mut out = io::stderr().lock();
                restore(&mut out, screen);
            }
            previous(info);
        }));
    });
}

/// Leaves the alternate screen (if entered), shows the cursor and leaves raw
/// mode, ignoring errors. Safe to repeat.
fn restore(mut out: &mut dyn Write, screen: Screen) {
    if screen == Screen::Alternate {
        let _ = queue!(&mut out, LeaveAlternateScreen);
    }
    let _ = queue!(&mut out, Show);
    let _ = out.flush();
    let _ = terminal::disable_raw_mode();
}

/// RAII over a raw-mode session on stderr: raw mode, hidden cursor, and
/// optionally the alternate screen.
///
/// Restores the terminal on `Drop`, and on panic through a process-wide
/// panic hook. Only one guard may be alive at a time.
pub struct TerminalGuard<'a> {
    out: &'a mut io::Stderr,
    screen: Screen,
}

impl<'a> TerminalGuard<'a> {
    /// Enters raw mode on `out` (and the alternate screen if asked), and hides
    /// the cursor. `out` is flushed first.
    ///
    /// # Errors
    ///
    /// [`UiError::Io`] when the terminal refuses, or when another guard is
    /// already alive.
    pub(crate) fn acquire(out: &'a mut io::Stderr, screen: Screen) -> Result<Self, UiError> {
        let io_err = |source| UiError::Io { source };
        out.flush().map_err(io_err)?;
        install_panic_hook();
        if ACTIVE
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
        {
            return Err(io_err(io::Error::other("the terminal is already in use")));
        }
        terminal::enable_raw_mode().map_err(io_err)?;
        // From here on `Drop` restores whatever was entered.
        let mut guard = Self { out, screen };
        *ACTIVE.lock().unwrap_or_else(PoisonError::into_inner) = Some(screen);
        let entered = if screen == Screen::Alternate {
            queue!(guard, EnterAlternateScreen, Hide)
        } else {
            queue!(guard, Hide)
        };
        entered.and_then(|()| guard.flush()).map_err(io_err)?;
        Ok(guard)
    }

    /// Waits up to `timeout` for a key press. `Ok(None)` on timeout.
    ///
    /// # Errors
    ///
    /// [`UiError::Io`].
    #[expect(
        clippy::unused_self,
        reason = "borrowing TerminalGuard proves raw mode is active"
    )]
    pub fn read_key(&mut self, timeout: Duration) -> Result<Option<KeyEvent>, UiError> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let polled = if event::poll(left).map_err(|source| UiError::Io { source })? {
                Some(event::read().map_err(|source| UiError::Io { source })?)
            } else {
                None
            };
            match polled {
                Some(Event::Key(key)) if key.kind != KeyEventKind::Release => {
                    return Ok(Some(key));
                }
                Some(_) if !left.is_zero() => {}
                _ => return Ok(None),
            }
        }
    }

    /// The terminal size as `(columns, rows)`; `(80, 24)` when the terminal
    /// reports a zero dimension (some ptys before their first resize).
    ///
    /// # Errors
    ///
    /// [`UiError::Io`].
    #[expect(
        clippy::unused_self,
        reason = "borrowing TerminalGuard ties size queries to the active session"
    )]
    pub fn size(&self) -> Result<(u16, u16), UiError> {
        match terminal::size() {
            Ok((w, h)) if w > 0 && h > 0 => Ok((w, h)),
            Ok(_) => Ok((80, 24)),
            Err(source) => Err(UiError::Io { source }),
        }
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
        ACTIVE.lock().unwrap_or_else(PoisonError::into_inner).take();
        restore(self.out, self.screen);
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
    /// borrows the Ui, so nothing else can draw meanwhile. Buffered stdout is
    /// flushed first.
    ///
    /// # Errors
    ///
    /// [`UiError::NotInteractive`] in `Plain` mode, [`UiError::BrokenPipe`]
    /// if pending stdout cannot be flushed because its reader went away, or
    /// [`UiError::Io`].
    pub fn terminal(&mut self, screen: Screen) -> Result<TerminalGuard<'_>, UiError> {
        if self.mode == Mode::Plain {
            return Err(UiError::NotInteractive {
                what: "this screen",
            });
        }
        self.flush()?;
        TerminalGuard::acquire(&mut self.err, screen)
    }
}
