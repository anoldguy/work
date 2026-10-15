//! [`TerminalGuard`]: the only way to get raw mode or the alternate screen.

use std::fmt;
use std::io::{self, Write};
use std::panic;
use std::sync::{Arc, Mutex, Once, PoisonError, TryLockError};
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

/// The process-wide terminal calls a guard makes.
pub(crate) trait Backend: Send + Sync {
    fn enable_raw(&self) -> io::Result<()>;
    fn disable_raw(&self) -> io::Result<()>;
    /// Waits up to `timeout` for any input event.
    fn poll(&self, timeout: Duration) -> io::Result<Option<Event>>;
    /// `(columns, rows)`.
    fn size(&self) -> io::Result<(u16, u16)>;
    /// Where the panic hook writes the restore sequence.
    fn panic_writer(&self) -> Box<dyn Write>;
}

pub(crate) struct Crossterm {
    err: io::Stderr,
}

impl Crossterm {
    pub(crate) const fn new(err: io::Stderr) -> Self {
        Self { err }
    }
}

impl Backend for Crossterm {
    fn enable_raw(&self) -> io::Result<()> {
        terminal::enable_raw_mode()
    }

    fn disable_raw(&self) -> io::Result<()> {
        terminal::disable_raw_mode()
    }

    fn poll(&self, timeout: Duration) -> io::Result<Option<Event>> {
        if event::poll(timeout)? {
            event::read().map(Some)
        } else {
            Ok(None)
        }
    }

    fn size(&self) -> io::Result<(u16, u16)> {
        terminal::size()
    }

    fn panic_writer(&self) -> Box<dyn Write> {
        Box::new(self.err.lock())
    }
}

/// The session the panic hook must restore, while a guard is alive.
struct Active {
    backend: Arc<dyn Backend>,
    screen: Screen,
}

/// Set by `acquire`, cleared by `Drop` (or taken by the panic hook).
static ACTIVE: Mutex<Option<Active>> = Mutex::new(None);

/// Installs (once per process) the panic hook that restores the active
/// guard session, if any, and then chains to the previous hook, so its
/// message is readable.
///
/// A panic on any thread restores the terminal, even from under a guard
/// owned by another thread; that guard's `Drop` then restores again,
/// harmlessly.
pub(super) fn install_panic_hook() {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let active = match ACTIVE.try_lock() {
                Ok(mut slot) => slot.take(),
                Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner().take(),
                Err(TryLockError::WouldBlock) => None,
            };
            if let Some(active) = active {
                let mut out = active.backend.panic_writer();
                restore(&mut *out, active.screen, &*active.backend);
            }
            previous(info);
        }));
    });
}

/// Leaves the alternate screen (if entered), shows the cursor and leaves raw
/// mode, ignoring errors. Safe to repeat.
fn restore(mut out: &mut dyn Write, screen: Screen, backend: &dyn Backend) {
    if screen == Screen::Alternate {
        let _ = queue!(&mut out, LeaveAlternateScreen);
    }
    let _ = queue!(&mut out, Show);
    let _ = out.flush();
    let _ = backend.disable_raw();
}

/// RAII over a raw-mode session on stderr: raw mode, hidden cursor, and
/// optionally the alternate screen.
///
/// Restores the terminal on `Drop`, and on panic through a process-wide
/// panic hook. Only one guard may be alive at a time.
pub struct TerminalGuard<'a> {
    out: &'a mut (dyn Write + Send),
    screen: Screen,
    backend: Arc<dyn Backend>,
}

impl<'a> TerminalGuard<'a> {
    /// Enters raw mode through `backend`, on `out` (and the alternate screen
    /// if asked), and hides the cursor. `out` is flushed first.
    ///
    /// # Errors
    ///
    /// [`UiError::Io`] when the terminal refuses, or when another guard is
    /// already alive.
    pub(crate) fn acquire(
        out: &'a mut (dyn Write + Send),
        screen: Screen,
        backend: Arc<dyn Backend>,
    ) -> Result<Self, UiError> {
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
        backend.enable_raw().map_err(io_err)?;
        // From here on `Drop` restores whatever was entered.
        let mut guard = Self {
            out,
            screen,
            backend,
        };
        *ACTIVE.lock().unwrap_or_else(PoisonError::into_inner) = Some(Active {
            backend: Arc::clone(&guard.backend),
            screen,
        });
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
    pub fn read_key(&mut self, timeout: Duration) -> Result<Option<KeyEvent>, UiError> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self
                .backend
                .poll(left)
                .map_err(|source| UiError::Io { source })?
            {
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
    pub fn size(&self) -> Result<(u16, u16), UiError> {
        match self.backend.size() {
            Ok((w, h)) if w > 0 && h > 0 => Ok((w, h)),
            Ok(_) => Ok((80, 24)),
            Err(source) => Err(UiError::Io { source }),
        }
    }
}

/// A writer that can wait for a key and report its size.
pub(crate) trait Term: Write {
    /// Waits up to `timeout` for a key press. `Ok(None)` on timeout.
    ///
    /// # Errors
    ///
    /// [`UiError::Io`].
    fn read_key(&mut self, timeout: Duration) -> Result<Option<KeyEvent>, UiError>;

    /// `(columns, rows)`.
    ///
    /// # Errors
    ///
    /// [`UiError::Io`].
    fn size(&self) -> Result<(u16, u16), UiError>;
}

impl Term for TerminalGuard<'_> {
    fn read_key(&mut self, timeout: Duration) -> Result<Option<KeyEvent>, UiError> {
        TerminalGuard::read_key(self, timeout)
    }

    fn size(&self) -> Result<(u16, u16), UiError> {
        TerminalGuard::size(self)
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
        restore(self.out, self.screen, &*self.backend);
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
        let backend = Arc::clone(&self.backend);
        self.terminal_with(screen, backend)
    }

    /// [`Ui::terminal`] over an explicit backend.
    pub(crate) fn terminal_with(
        &mut self,
        screen: Screen,
        backend: Arc<dyn Backend>,
    ) -> Result<TerminalGuard<'_>, UiError> {
        if self.mode == Mode::Plain {
            return Err(UiError::NotInteractive {
                what: "this screen",
            });
        }
        self.flush()?;
        TerminalGuard::acquire(&mut *self.err, screen, backend)
    }
}
