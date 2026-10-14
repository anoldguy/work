//! `Ui::with_progress`: runs work on a scoped thread while the calling
//! thread draws the live display.
//!
//! The drawn area is registered in [`AREA`] so the process panic hook (see
//! `ui::terminal`) can erase it before the hook's one-line message is
//! printed, whichever thread panics. The cursor is never hidden, so an
//! interrupt (Ctrl-C) or an abort leaves at worst a stale frame, never an
//! invisible cursor.

use std::io::{self, Write};
use std::mem;
use std::panic;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use crossterm::cursor::MoveToPreviousLine;
use crossterm::queue;
use crossterm::terminal::{self, Clear, ClearType};

use crate::progress::{ProgressSink, ProgressView, UiEvent};
use crate::ui::{Mode, Ui, terminal::install_panic_hook};

/// How often the live display is redrawn (and the spinner advances).
const REDRAW: Duration = Duration::from_millis(80);

/// Size used when the terminal cannot report one, or reports a zero.
const FALLBACK_SIZE: (u16, u16) = (80, 24);

/// Where the panic hook writes its erase sequence.
type PanicOut = Box<dyn Fn() -> Box<dyn Write> + Send>;

/// The live area, shared with the panic hook.
struct Area {
    /// Rows of the last frame, all above the cursor.
    drawn: u16,
    /// Set by the panic hook after it erased the area: stop drawing.
    suspended: bool,
    /// The hook's writer: stderr in production.
    panic_out: PanicOut,
}

/// The registered live area while a `Tty` `with_progress` runs. Each frame
/// is written with the lock held, so the hook never lands mid-frame.
static AREA: Mutex<Option<Area>> = Mutex::new(None);

/// Moves to the top of a `drawn`-row block and clears it and everything
/// below. Best effort: a broken stderr loses the display.
fn erase(mut out: &mut dyn Write, drawn: u16) {
    if drawn > 0 {
        let _ = queue!(&mut out, MoveToPreviousLine(drawn));
    }
    let _ = queue!(&mut out, Clear(ClearType::FromCursorDown));
}

/// Locks [`AREA`], ignoring poison (the state stays consistent).
fn area() -> std::sync::MutexGuard<'static, Option<Area>> {
    AREA.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Called by the panic hook: erases the live area, if any, and stops
/// further drawing, so the hook's message lands where the area was.
pub(super) fn erase_after_panic() {
    let mut slot = area();
    if let Some(live) = slot.as_mut()
        && !live.suspended
    {
        let mut out = (live.panic_out)();
        erase(&mut *out, live.drawn);
        let _ = out.flush();
        live.drawn = 0;
        live.suspended = true;
    }
}

/// Replaces the registered area's frame with `lines` on `err`, unless the
/// panic hook has suspended it.
fn draw(err: &mut dyn Write, lines: &[String]) {
    let mut slot = area();
    let Some(live) = slot.as_mut().filter(|live| !live.suspended) else {
        return;
    };
    erase(err, live.drawn);
    for line in lines {
        let _ = writeln!(err, "{line}");
    }
    let _ = err.flush();
    live.drawn = u16::try_from(lines.len()).unwrap_or(u16::MAX);
}

/// Injected timing and terminal access, so tests run without a terminal
/// and without waiting on the redraw clock.
struct Seams<S> {
    interval: Duration,
    size: S,
    panic_out: PanicOut,
}

/// Ends a `with_progress` call, on return or unwind: erases the area (unless
/// the panic hook already did), prints the summary (when set), then flushes
/// the deferred warnings.
struct Live<'a> {
    ui: &'a mut Ui,
    tty: bool,
    summary: Option<String>,
    /// The event receiver; warnings still queued here (after a drawing
    /// panic) are flushed too.
    rx: mpsc::Receiver<UiEvent>,
}

impl Drop for Live<'_> {
    fn drop(&mut self) {
        if self.tty {
            if let Some(live) = area().take()
                && !live.suspended
            {
                erase(&mut *self.ui.err, live.drawn);
            }
            if let Some(summary) = self.summary.take() {
                let _ = writeln!(self.ui.err, "{summary}");
            }
            let _ = self.ui.err.flush();
        }
        let queued = self.rx.try_iter().filter_map(|event| match event {
            UiEvent::Warn(message) => Some(message),
            _ => None,
        });
        self.ui.deferred.extend(queued);
        for message in mem::take(&mut self.ui.deferred) {
            self.ui.warn(&message);
        }
    }
}

impl Ui {
    /// Runs `work` on a scoped thread with a [`ProgressSink`], while this
    /// thread folds its events into a `ProgressView` and (in `Tty` mode)
    /// redraws it in place on stderr every 80 ms, until the work's thread
    /// has finished. Then clears the area, prints the summary line (`Tty`
    /// only), and flushes deferred warnings (both modes). `title` is shown
    /// until the work sends a `Header`. Events sent after the work returns
    /// (from a sink clone that escaped it) are dropped.
    ///
    /// # Panics
    ///
    /// If `work` panics, the display is cleared (before the panic message
    /// is printed) and warnings flushed, then the panic is resumed. A panic
    /// while drawing clears the display and flushes warnings the same way.
    pub fn with_progress<R: Send>(
        &mut self,
        title: &str,
        work: impl FnOnce(&ProgressSink) -> R + Send,
    ) -> R {
        let seams = Seams {
            interval: REDRAW,
            size: || match terminal::size() {
                Ok((w, h)) if w > 0 && h > 0 => (w, h),
                _ => FALLBACK_SIZE,
            },
            panic_out: Box::new(|| Box::new(io::stderr())),
        };
        self.progress_with(title, work, seams)
    }

    /// [`Ui::with_progress`] over explicit [`Seams`].
    fn progress_with<R: Send>(
        &mut self,
        title: &str,
        work: impl FnOnce(&ProgressSink) -> R + Send,
        seams: Seams<impl Fn() -> (u16, u16)>,
    ) -> R {
        let tty = self.mode == Mode::Tty;
        let mut view = ProgressView::new(title, self.color);
        let (sink, rx) = ProgressSink::channel();
        let mut live = Live {
            ui: self,
            tty,
            summary: None,
            rx,
        };
        let ui = &mut *live.ui;
        let rx = &live.rx;
        let mut tick: u8 = 0;
        let mut redraw = |view: &ProgressView, err: &mut dyn Write| {
            if tty {
                let (w, h) = (seams.size)();
                draw(err, &view.lines(w, h, tick));
                tick = tick.wrapping_add(1);
            }
        };

        if tty {
            // Anything already printed must land above the display.
            ui.flush_for_stderr();
            install_panic_hook();
            *area() = Some(Area {
                drawn: 0,
                suspended: false,
                panic_out: seams.panic_out,
            });
            redraw(&view, &mut *ui.err);
        }

        let err = &mut *ui.err;
        let deferred = &mut ui.deferred;
        let mut fold = |view: &mut ProgressView, event: UiEvent| match event {
            UiEvent::Warn(message) => deferred.push(message),
            other => view.apply(other),
        };
        let joined = thread::scope(|scope| {
            let worker = scope.spawn(move || work(&sink));
            let mut last_draw = Instant::now();
            loop {
                let wait = seams.interval.saturating_sub(last_draw.elapsed());
                match rx.recv_timeout(wait) {
                    Ok(event) => fold(&mut view, event),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
                // A sink clone may outlive the work, so the channel cannot
                // be trusted to disconnect: the thread finishing is the end.
                if worker.is_finished() {
                    break;
                }
                if last_draw.elapsed() >= seams.interval {
                    redraw(&view, err);
                    last_draw = Instant::now();
                }
            }
            // Everything the work sent before finishing is queued by now.
            for event in rx.try_iter() {
                fold(&mut view, event);
            }
            worker.join()
        });

        match joined {
            Ok(result) => {
                if tty {
                    live.summary = view.summary_line();
                }
                drop(live);
                result
            }
            Err(payload) => {
                drop(live);
                panic::resume_unwind(payload)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::panic::{self, AssertUnwindSafe};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, PoisonError, mpsc};
    use std::thread;
    use std::time::Duration;

    use super::{FALLBACK_SIZE, REDRAW, Seams};
    use crate::progress::{Header, Noun, ProgressSink, TaskOutcome, UiEvent, Verb};
    use crate::testing::{ScriptedInteraction, SharedBuf};
    use crate::ui::terminal::fake::{FakeBackend, HOOK_TEST_LOCK, with_hook_line};
    use crate::ui::{Mode, Screen, TerminalGuard, Ui};

    fn ui(mode: Mode) -> (Ui, SharedBuf, SharedBuf) {
        let (out, err) = (SharedBuf::new(), SharedBuf::new());
        let ui = Ui::for_test_mode(
            mode,
            Box::new(out.clone()),
            Box::new(err.clone()),
            Box::new(ScriptedInteraction::new(false, vec![])),
        );
        (ui, out, err)
    }

    /// Redraw on every loop turn, at 80×24; the hook erases into `err`.
    fn seams(err: &SharedBuf) -> Seams<impl Fn() -> (u16, u16)> {
        let err = err.clone();
        Seams {
            interval: Duration::ZERO,
            size: || FALLBACK_SIZE,
            panic_out: Box::new(move || Box::new(err.clone())),
        }
    }

    /// Replays `bytes` through a tiny VT (text, `\n`, `ESC[nF`, `ESC[J`;
    /// other CSI sequences ignored) and returns the non-empty screen rows.
    fn screen(bytes: &str) -> Vec<String> {
        let mut rows: Vec<Vec<char>> = vec![Vec::new()];
        let (mut row, mut col) = (0usize, 0usize);
        let mut chars = bytes.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\x1b' => {
                    assert_eq!(chars.next(), Some('['), "CSI only: {bytes:?}");
                    let mut param = String::new();
                    let fin = loop {
                        let c = chars.next().expect("complete CSI");
                        if c.is_ascii_alphabetic() {
                            break c;
                        }
                        param.push(c);
                    };
                    match fin {
                        'F' => {
                            row = row.saturating_sub(param.parse().unwrap_or(1));
                            col = 0;
                        }
                        'J' => {
                            rows[row].truncate(col);
                            rows.truncate(row + 1);
                        }
                        _ => {}
                    }
                }
                '\n' => {
                    row += 1;
                    col = 0;
                    if rows.len() <= row {
                        rows.push(Vec::new());
                    }
                }
                '\r' => col = 0,
                c => {
                    let line = &mut rows[row];
                    if col < line.len() {
                        line[col] = c;
                    } else {
                        line.push(c);
                    }
                    col += 1;
                }
            }
        }
        rows.into_iter()
            .map(|r| r.into_iter().collect::<String>())
            .filter(|r| !r.is_empty())
            .collect()
    }

    fn header(total: u32) -> UiEvent {
        UiEvent::Header(Header {
            verb: Verb::Updating,
            total,
            noun: Noun::Sources,
            domains: None,
        })
    }

    /// Two orgs: one fetches 3 repos, one fails; one warning.
    fn two_orgs(sink: &ProgressSink) -> &'static str {
        sink.send(header(2));
        for (id, label) in [(0, "github.com/acme"), (1, "github.com/widgets")] {
            sink.send(UiEvent::TaskStarted {
                id,
                label: label.to_owned(),
            });
        }
        sink.warn("org widgets failed: HTTP 502");
        sink.send(UiEvent::TaskFinished {
            id: 0,
            outcome: TaskOutcome::Ok { repos: Some(3) },
        });
        sink.send(UiEvent::TaskFinished {
            id: 1,
            outcome: TaskOutcome::Failed {
                reason: "HTTP 502".to_owned(),
            },
        });
        "done"
    }

    #[test]
    fn plain_mode_writes_nothing_but_warnings() {
        let (mut ui, out, err) = ui(Mode::Plain);
        let result = ui.progress_with("Updating…", two_orgs, seams(&err));
        assert_eq!(result, "done");
        assert_eq!(out.contents(), "");
        assert_eq!(err.contents(), "warning: org widgets failed: HTTP 502\n");
    }

    #[test]
    fn tty_mode_clears_then_prints_the_summary_and_deferred_warnings() {
        let _lock = HOOK_TEST_LOCK
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let (mut ui, out, err) = ui(Mode::Tty);
        let result = ui.progress_with("Updating…", two_orgs, seams(&err));
        assert_eq!(result, "done");
        assert_eq!(out.contents(), "");
        let bytes = err.contents();
        assert!(bytes.starts_with("\x1b[JUpdating…\n"), "{bytes:?}");
        assert_eq!(bytes.matches("warning:").count(), 1, "deferred: {bytes:?}");
        assert_eq!(
            screen(&bytes),
            [
                "✓ 2 sources · 3 repos · 1 failed",
                "warning: org widgets failed: HTTP 502"
            ]
        );
    }

    #[test]
    fn a_multi_line_frame_is_erased_in_full() {
        let _lock = HOOK_TEST_LOCK
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let (mut ui, _out, err) = ui(Mode::Tty);
        let seen = err.clone();
        ui.progress_with(
            "t",
            move |sink| {
                sink.send(header(3));
                for id in 0..3 {
                    sink.send(UiEvent::TaskStarted {
                        id,
                        label: format!("acme/{id}"),
                    });
                }
                // Wait (no clock) until a four-row frame is on screen.
                while !seen.contents().contains("acme/2") {
                    thread::yield_now();
                }
            },
            seams(&err),
        );
        assert!(err.contents().contains("\x1b[4F\x1b[J"));
        assert_eq!(screen(&err.contents()), Vec::<String>::new());
    }

    #[test]
    fn tty_mode_flushes_stdout_before_drawing() {
        let _lock = HOOK_TEST_LOCK
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let (mut ui, out, err) = ui(Mode::Tty);
        ui.stdout_line("earlier output").expect("write");
        let seen = ui.progress_with("t", |_| out.contents(), seams(&err));
        assert_eq!(seen, "earlier output\n");
    }

    #[test]
    fn returns_once_every_sender_is_gone() {
        let (mut ui, _out, err) = ui(Mode::Plain);
        let total = ui.with_progress("t", |sink| {
            thread::scope(|scope| {
                let handles: Vec<_> = (0..4)
                    .map(|i| scope.spawn(move || sink.warn(format!("w{i}"))))
                    .collect();
                handles
                    .into_iter()
                    .map(|h| h.join().map_or(0, |()| 1))
                    .sum::<u32>()
            })
        });
        assert_eq!(total, 4);
        assert_eq!(err.contents().lines().count(), 4);
    }

    #[test]
    fn returns_even_when_a_sink_clone_escapes_the_work() {
        let (done_tx, done_rx) = mpsc::channel();
        thread::spawn(move || {
            let (mut ui, _out, _err) = ui(Mode::Plain);
            let escaped: ProgressSink = ui.with_progress("t", ProgressSink::clone);
            escaped.warn("too late");
            done_tx.send(()).ok();
        });
        // A watchdog, not a sleep: fails instead of hanging forever.
        assert_eq!(done_rx.recv_timeout(REDRAW * 250), Ok(()));
    }

    #[test]
    fn a_worker_panic_line_survives_and_the_area_is_erased() {
        let _lock = HOOK_TEST_LOCK
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let (mut ui, _out, err) = ui(Mode::Tty);
        let seen = err.clone();
        let caught = with_hook_line(&err, || {
            panic::catch_unwind(AssertUnwindSafe(|| {
                ui.progress_with(
                    "t",
                    move |sink| {
                        sink.send(header(3));
                        sink.warn("before the crash");
                        while !seen.contents().contains("Updating 3 sources") {
                            thread::yield_now();
                        }
                        panic!("worker boom");
                    },
                    seams(&err),
                )
            }))
        });
        let payload = caught.expect_err("panic resumed");
        assert_eq!(payload.downcast_ref::<&str>(), Some(&"worker boom"));
        assert_eq!(
            screen(&err.contents()),
            [
                "work: internal error: worker boom (please report this)",
                "warning: before the crash"
            ]
        );
    }

    #[test]
    fn a_drawing_panic_erases_the_area_and_flushes_warnings() {
        let _lock = HOOK_TEST_LOCK
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let (mut ui, _out, err) = ui(Mode::Tty);
        let calls = Cell::new(0);
        let probing = AtomicBool::new(false);
        let hook_err = err.clone();
        let seams = Seams {
            interval: Duration::ZERO,
            size: || {
                calls.set(calls.get() + 1);
                if calls.get() == 2 {
                    probing.store(true, Ordering::SeqCst);
                    panic!("size probe");
                }
                FALLBACK_SIZE
            },
            panic_out: Box::new(move || Box::new(hook_err.clone())),
        };
        let caught = with_hook_line(&err, || {
            panic::catch_unwind(AssertUnwindSafe(|| {
                ui.progress_with(
                    "t",
                    |sink| {
                        sink.warn("queued");
                        while !probing.load(Ordering::SeqCst) {
                            thread::yield_now();
                        }
                    },
                    seams,
                );
            }))
        });
        assert!(caught.is_err());
        assert_eq!(
            screen(&err.contents()),
            [
                "work: internal error: size probe (please report this)",
                "warning: queued"
            ]
        );
    }

    #[test]
    fn a_panic_under_a_guard_restores_the_terminal_and_the_progress_area() {
        let _lock = HOOK_TEST_LOCK
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let (mut ui, _out, err) = ui(Mode::Tty);
        let backend = Arc::new(FakeBackend::default());
        let mut screen_out = SharedBuf::new();
        let caught = panic::catch_unwind(AssertUnwindSafe(|| {
            let _guard =
                TerminalGuard::acquire_with(&mut screen_out, Screen::Alternate, backend.clone())
                    .expect("acquire");
            ui.progress_with(
                "t",
                |sink| {
                    sink.warn("lost connection");
                    panic!("boom");
                },
                seams(&err),
            )
        }));
        assert!(caught.is_err());
        assert_eq!(
            backend.log(),
            ["raw on", "panic restore", "raw off", "raw off"]
        );
        assert!(
            backend
                .hook_out
                .contents()
                .ends_with("\x1b[?1049l\x1b[?25h")
        );
        assert_eq!(screen(&err.contents()), ["warning: lost connection"]);
    }
}
