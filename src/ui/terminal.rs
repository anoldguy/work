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
    /// Below the cursor in the normal screen (pickers, prompts).
    Inline,
    /// The alternate screen, leaving scrollback untouched (`about`).
    Alternate,
}

/// The process-wide terminal calls a guard makes, behind a seam so tests
/// can drive the guard without a real terminal.
pub(crate) trait Backend: Send + Sync {
    /// Enters raw mode.
    fn enable_raw(&self) -> io::Result<()>;
    /// Leaves raw mode.
    fn disable_raw(&self) -> io::Result<()>;
    /// Waits up to `timeout` for any input event.
    fn poll(&self, timeout: Duration) -> io::Result<Option<Event>>;
    /// `(columns, rows)`.
    fn size(&self) -> io::Result<(u16, u16)>;
    /// Where the panic hook writes the restore sequence.
    fn panic_writer(&self) -> Box<dyn Write>;
}

/// The real terminal, through crossterm.
struct Crossterm;

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
        Box::new(io::stderr())
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
/// guard session and erases a live progress area, if any, and then chains
/// to the previous hook (`main`'s), so its one-line message is readable.
///
/// The hook is process-wide: a panic on any thread restores the terminal,
/// even from under a guard owned by another thread (that guard's `Drop`
/// then restores again, harmlessly). Lib tests that hold a guard or
/// panic on purpose serialise on `fake::HOOK_TEST_LOCK`; tests elsewhere
/// that panic can still race them under `cargo test`, which nextest's
/// process-per-test model rules out.
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
            super::progress::erase_after_panic();
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
/// Restores the terminal on `Drop`, and on panic through the process panic
/// hook installed by [`TerminalGuard::acquire`], which restores whichever
/// guard is alive. Only one guard may be alive at a time. Writes go to the borrowed stderr
/// writer through the `Write` impl.
pub struct TerminalGuard<'a> {
    out: &'a mut (dyn Write + Send),
    screen: Screen,
    backend: Arc<dyn Backend>,
}

impl<'a> TerminalGuard<'a> {
    /// Enters raw mode on `out` (and the alternate screen if asked) and hides
    /// the cursor. The process stdout handle and `out` are flushed first;
    /// a caller holding its own stdout buffer (the `Ui`) flushes that before
    /// calling.
    ///
    /// # Errors
    ///
    /// [`UiError::Io`] when the terminal refuses, or when another guard is
    /// already alive.
    pub fn acquire(out: &'a mut (dyn Write + Send), screen: Screen) -> Result<Self, UiError> {
        Self::acquire_with(out, screen, Arc::new(Crossterm))
    }

    /// [`TerminalGuard::acquire`] over an explicit backend.
    pub(crate) fn acquire_with(
        out: &'a mut (dyn Write + Send),
        screen: Screen,
        backend: Arc<dyn Backend>,
    ) -> Result<Self, UiError> {
        let io_err = |source| UiError::Io { source };
        io::stdout().flush().map_err(io_err)?;
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

/// The terminal as the interaction and splash loops see it: a writer that
/// can wait for a key and report its size. [`TerminalGuard`] is the real
/// one; tests drive the loops through `fake::FakeTerm`.
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
        self.terminal_with(screen, Arc::new(Crossterm))
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
        TerminalGuard::acquire_with(&mut *self.err, screen, backend)
    }
}

/// A scripted [`Backend`] and [`Term`] for tests, and the lock that keeps
/// tests that panic or hold a guard from seeing each other through the
/// global hook.
#[cfg(test)]
pub(crate) mod fake {
    use std::cell::Cell;
    use std::collections::VecDeque;
    use std::io::{self, Write};
    use std::panic;
    use std::rc::Rc;
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::Duration;

    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

    use super::{Backend, Term};
    use crate::testing::SharedBuf;
    use crate::ui::UiError;

    /// A scripted [`Term`]: replays a script of keys and timeouts, records
    /// what was written, and keeps a fake clock.
    ///
    /// A timeout advances the clock by the full wait and applies the next
    /// queued resize, as if the window had changed meanwhile. Running out
    /// of script panics, so a loop that never finishes fails fast.
    #[derive(Debug)]
    pub(crate) struct FakeTerm {
        script: VecDeque<Option<KeyEvent>>,
        resizes: VecDeque<(u16, u16)>,
        size: (u16, u16),
        out: Vec<u8>,
        elapsed: Rc<Cell<Duration>>,
    }

    impl FakeTerm {
        /// A `size` terminal with an empty script.
        pub(crate) fn new(size: (u16, u16)) -> Self {
            Self {
                script: VecDeque::new(),
                resizes: VecDeque::new(),
                size,
                out: Vec::new(),
                elapsed: Rc::new(Cell::new(Duration::ZERO)),
            }
        }

        /// Appends `n` timeouts to the script.
        pub(crate) fn timeouts(mut self, n: usize) -> Self {
            self.script.extend(std::iter::repeat_n(None, n));
            self
        }

        /// Appends unmodified presses of `codes` to the script.
        pub(crate) fn keys(mut self, codes: &[KeyCode]) -> Self {
            self.script.extend(
                codes
                    .iter()
                    .map(|&code| Some(KeyEvent::new(code, KeyModifiers::NONE))),
            );
            self
        }

        /// Appends Ctrl+`c` to the script.
        pub(crate) fn ctrl(mut self, c: char) -> Self {
            self.script
                .push_back(Some(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)));
            self
        }

        /// Queues a resize, applied at the next timeout.
        pub(crate) fn resize_on_timeout(mut self, size: (u16, u16)) -> Self {
            self.resizes.push_back(size);
            self
        }

        /// The fake clock: the time spent waiting out timeouts so far.
        pub(crate) fn clock(&self) -> impl Fn() -> Duration + use<> {
            let elapsed = Rc::clone(&self.elapsed);
            move || elapsed.get()
        }

        /// Everything written, lossily decoded.
        pub(crate) fn output(&self) -> String {
            String::from_utf8_lossy(&self.out).into_owned()
        }

        /// The number of bytes written so far.
        pub(crate) fn written(&self) -> usize {
            self.out.len()
        }
    }

    impl Write for FakeTerm {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.out.write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Term for FakeTerm {
        fn read_key(&mut self, timeout: Duration) -> Result<Option<KeyEvent>, UiError> {
            let step = self.script.pop_front().expect("the script ran out of keys");
            if step.is_none() {
                self.elapsed.set(self.elapsed.get() + timeout);
                if let Some(size) = self.resizes.pop_front() {
                    self.size = size;
                }
            }
            Ok(step)
        }

        fn size(&self) -> Result<(u16, u16), UiError> {
            Ok(self.size)
        }
    }

    /// Runs `f` with the panic hook extended to append `main`'s one-line
    /// message (`work: internal error: <msg> (please report this)`) to
    /// `buf` after the crate's own hook has run, as `main`'s hook would
    /// print it. Caller holds [`HOOK_TEST_LOCK`].
    pub(crate) fn with_hook_line<T>(buf: &SharedBuf, f: impl FnOnce() -> T) -> T {
        super::install_panic_hook();
        let ours: Arc<PanicHook> = Arc::new(panic::take_hook());
        let chained = Arc::clone(&ours);
        let out = buf.clone();
        panic::set_hook(Box::new(move |info| {
            chained(info);
            let payload = info.payload();
            let message = payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("unknown panic");
            let _ = writeln!(
                out.clone(),
                "work: internal error: {message} (please report this)"
            );
        }));
        let result = f();
        drop(panic::take_hook());
        panic::set_hook(Box::new(move |info| ours(info)));
        result
    }

    type PanicHook = Box<dyn Fn(&panic::PanicHookInfo<'_>) + Sync + Send + 'static>;

    /// Held by every test that installs a guard or panics on purpose.
    pub(crate) static HOOK_TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Records raw-mode calls in `log`; the panic hook writes to `hook_out`.
    #[derive(Debug, Default)]
    pub(crate) struct FakeBackend {
        pub log: Mutex<Vec<&'static str>>,
        pub events: Mutex<VecDeque<Event>>,
        pub hook_out: SharedBuf,
        pub refuse_raw: bool,
        pub size: Option<(u16, u16)>,
    }

    impl FakeBackend {
        pub(crate) fn log(&self) -> Vec<&'static str> {
            self.log
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }

        fn record(&self, what: &'static str) {
            self.log
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(what);
        }
    }

    impl Backend for FakeBackend {
        fn enable_raw(&self) -> io::Result<()> {
            if self.refuse_raw {
                return Err(io::Error::other("not a terminal"));
            }
            self.record("raw on");
            Ok(())
        }

        fn disable_raw(&self) -> io::Result<()> {
            self.record("raw off");
            Ok(())
        }

        fn poll(&self, _timeout: Duration) -> io::Result<Option<Event>> {
            Ok(self
                .events
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front())
        }

        fn size(&self) -> io::Result<(u16, u16)> {
            Ok(self.size.unwrap_or((80, 24)))
        }

        fn panic_writer(&self) -> Box<dyn Write> {
            self.record("panic restore");
            Box::new(self.hook_out.clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::io::Write;
    use std::panic::{self, AssertUnwindSafe};
    use std::sync::{Arc, PoisonError};
    use std::time::Duration;

    use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    use super::fake::{FakeBackend, HOOK_TEST_LOCK};
    use super::{Screen, TerminalGuard};
    use crate::testing::{ScriptedInteraction, SharedBuf};
    use crate::ui::{Mode, Ui, UiError};

    const ENTER_ALT: &str = "\x1b[?1049h";
    const LEAVE_ALT: &str = "\x1b[?1049l";
    const HIDE: &str = "\x1b[?25l";
    const SHOW: &str = "\x1b[?25h";

    #[test]
    fn inline_guard_hides_the_cursor_and_restores_on_drop() {
        let _lock = HOOK_TEST_LOCK
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let backend = Arc::new(FakeBackend::default());
        let buf = SharedBuf::new();
        let mut out = buf.clone();
        {
            let mut guard = TerminalGuard::acquire_with(&mut out, Screen::Inline, backend.clone())
                .expect("acquire");
            write!(guard, "frame").expect("write");
            assert_eq!(guard.size().expect("size"), (80, 24));
            assert_eq!(backend.log(), ["raw on"]);
        }
        assert_eq!(buf.contents(), format!("{HIDE}frame{SHOW}"));
        assert_eq!(backend.log(), ["raw on", "raw off"]);
    }

    #[test]
    fn alternate_guard_enters_and_leaves_the_alternate_screen() {
        let _lock = HOOK_TEST_LOCK
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let backend = Arc::new(FakeBackend::default());
        let buf = SharedBuf::new();
        let mut out = buf.clone();
        drop(
            TerminalGuard::acquire_with(&mut out, Screen::Alternate, backend.clone())
                .expect("acquire"),
        );
        assert_eq!(
            buf.contents(),
            format!("{ENTER_ALT}{HIDE}{LEAVE_ALT}{SHOW}")
        );
        assert_eq!(backend.log(), ["raw on", "raw off"]);
    }

    #[test]
    fn a_panic_restores_through_the_hook_and_again_on_drop() {
        let _lock = HOOK_TEST_LOCK
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let backend = Arc::new(FakeBackend::default());
        let mut out = SharedBuf::new();
        let caught = panic::catch_unwind(AssertUnwindSafe(|| {
            let _guard = TerminalGuard::acquire_with(&mut out, Screen::Alternate, backend.clone())
                .expect("acquire");
            panic!("boom");
        }));
        assert!(caught.is_err());
        assert_eq!(
            backend.log(),
            ["raw on", "panic restore", "raw off", "raw off"]
        );
        assert_eq!(backend.hook_out.contents(), format!("{LEAVE_ALT}{SHOW}"));
        assert!(out.contents().ends_with(&format!("{LEAVE_ALT}{SHOW}")));
    }

    #[test]
    fn the_hook_does_nothing_once_the_guard_is_gone() {
        let _lock = HOOK_TEST_LOCK
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let backend = Arc::new(FakeBackend::default());
        let mut out = SharedBuf::new();
        drop(
            TerminalGuard::acquire_with(&mut out, Screen::Inline, backend.clone())
                .expect("acquire"),
        );
        let caught = panic::catch_unwind(|| panic!("later"));
        assert!(caught.is_err());
        assert_eq!(backend.log(), ["raw on", "raw off"]);
    }

    #[test]
    fn a_second_live_guard_is_refused() {
        let _lock = HOOK_TEST_LOCK
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let backend = Arc::new(FakeBackend::default());
        let (mut first, mut second) = (SharedBuf::new(), SharedBuf::new());
        let guard = TerminalGuard::acquire_with(&mut first, Screen::Inline, backend.clone())
            .expect("acquire");
        assert_matches!(
            TerminalGuard::acquire_with(&mut second, Screen::Inline, backend.clone()),
            Err(UiError::Io { .. })
        );
        drop(guard);
        assert_eq!(backend.log(), ["raw on", "raw off"]);
        assert_eq!(second.contents(), "");
    }

    #[test]
    fn a_zero_size_falls_back_to_80_by_24() {
        let _lock = HOOK_TEST_LOCK
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let backend = Arc::new(FakeBackend {
            size: Some((0, 0)),
            ..FakeBackend::default()
        });
        let mut out = SharedBuf::new();
        let guard =
            TerminalGuard::acquire_with(&mut out, Screen::Inline, backend).expect("acquire");
        assert_eq!(guard.size().expect("size"), (80, 24));
    }

    #[test]
    fn a_refused_raw_mode_is_an_io_error() {
        let _lock = HOOK_TEST_LOCK
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let backend = Arc::new(FakeBackend {
            refuse_raw: true,
            ..FakeBackend::default()
        });
        let buf = SharedBuf::new();
        let mut out = buf.clone();
        assert_matches!(
            TerminalGuard::acquire_with(&mut out, Screen::Inline, backend),
            Err(UiError::Io { .. })
        );
        assert_eq!(buf.contents(), "");
    }

    #[test]
    fn read_key_returns_presses_skips_other_events_and_times_out() {
        let _lock = HOOK_TEST_LOCK
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let press = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        let mut release = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        let backend = Arc::new(FakeBackend::default());
        backend
            .events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend([Event::Resize(1, 1), Event::Key(release), Event::Key(press)]);
        let mut out = SharedBuf::new();
        let mut guard =
            TerminalGuard::acquire_with(&mut out, Screen::Inline, backend).expect("acquire");
        let timeout = Duration::from_secs(60);
        assert_eq!(guard.read_key(timeout).expect("read"), Some(press));
        assert_eq!(guard.read_key(Duration::ZERO).expect("read"), None);
    }

    #[test]
    fn plain_ui_has_no_terminal() {
        let mut ui = Ui::for_test(
            Box::new(SharedBuf::new()),
            Box::new(SharedBuf::new()),
            Box::new(ScriptedInteraction::new(false, vec![])),
        );
        assert_matches!(
            ui.terminal(Screen::Inline),
            Err(UiError::NotInteractive { .. })
        );
    }

    #[test]
    fn tty_ui_flushes_stdout_before_taking_the_terminal() {
        let _lock = HOOK_TEST_LOCK
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let (out, err) = (SharedBuf::new(), SharedBuf::new());
        let mut ui = Ui::for_test_mode(
            Mode::Tty,
            Box::new(out.clone()),
            Box::new(err.clone()),
            Box::new(ScriptedInteraction::new(false, vec![])),
        );
        ui.stdout_line("earlier").expect("write");
        let backend = Arc::new(FakeBackend::default());
        let guard = ui.terminal_with(Screen::Inline, backend).expect("terminal");
        assert_eq!(out.contents(), "earlier\n");
        drop(guard);
        assert_eq!(err.contents(), format!("{HIDE}{SHOW}"));
    }
}
