//! [`TtyInteraction`]: pickers and prompts on the real terminal, and
//! `Ui::interact`.

use std::fmt::{self, Write as _};
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use crate::text::clip;
use crate::ui::picker::{PickerKey, PickerState, PickerStep};
use crate::ui::prompt::{Confirm, LineInput, Step};
use crate::ui::terminal::{Backend, Term};
use crate::ui::{Interaction, Screen, TerminalGuard, Ui, UiError};

/// How long one `read_key` waits before the loop re-reads the terminal size
/// (and redraws if it changed).
const KEY_WAIT: Duration = Duration::from_millis(500);

/// The most picker lines (filter line plus items) drawn below the prompt.
const MAX_PICKER_ROWS: u16 = 12;

/// Show the cursor (the guard hides it, and restores it on drop).
const SHOW_CURSOR: &str = "\x1b[?25h";

/// The real [`Interaction`]: renders `picker::PickerState` and the prompts
/// on its own stderr handle inside a `TerminalGuard` (`Screen::Inline`).
///
/// A picker over no items could only be escaped, so `pick_one` and
/// `pick_many` with empty `items` return `Ok(None)` (cancelled) at once.
pub struct TtyInteraction {
    err: Box<dyn Write + Send>,
    interactive: bool,
    backend: Arc<dyn Backend>,
}

impl TtyInteraction {
    /// Draws on `err` through `backend`; `interactive` says whether there is
    /// a terminal to draw on.
    pub(crate) fn new(
        err: Box<dyn Write + Send>,
        interactive: bool,
        backend: Arc<dyn Backend>,
    ) -> Self {
        Self {
            err,
            interactive,
            backend,
        }
    }

    /// [`UiError::NotInteractive`] unless there is a terminal.
    const fn require_terminal(&self, what: &'static str) -> Result<(), UiError> {
        if self.interactive {
            Ok(())
        } else {
            Err(UiError::NotInteractive { what })
        }
    }

    /// Takes over the terminal for one picker or prompt.
    fn guard(&mut self, what: &'static str) -> Result<TerminalGuard<'_>, UiError> {
        self.require_terminal(what)?;
        TerminalGuard::acquire(&mut *self.err, Screen::Inline, Arc::clone(&self.backend))
    }
}

impl fmt::Debug for TtyInteraction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TtyInteraction")
            .field("interactive", &self.interactive)
            .finish_non_exhaustive()
    }
}

impl Interaction for TtyInteraction {
    fn is_interactive(&self) -> bool {
        self.interactive
    }

    fn pick_one(&mut self, prompt: &str, items: &[&str]) -> Result<Option<usize>, UiError> {
        self.require_terminal("the picker")?;
        if items.is_empty() {
            return Ok(None);
        }
        let mut guard = self.guard("the picker")?;
        let mut state = PickerState::new(items, None);
        let step = run_picker(&mut guard, prompt, &mut state)?;
        Ok(match step {
            PickerStep::Chosen(chosen) => chosen.first().copied(),
            _ => None,
        })
    }

    fn pick_many(
        &mut self,
        prompt: &str,
        items: &[&str],
        selected: &[bool],
    ) -> Result<Option<Vec<bool>>, UiError> {
        self.require_terminal("the picker")?;
        if items.is_empty() {
            return Ok(None);
        }
        let mut guard = self.guard("the picker")?;
        let mut state = PickerState::new(items, Some(selected.to_vec()));
        let step = run_picker(&mut guard, prompt, &mut state)?;
        Ok(match step {
            PickerStep::Chosen(chosen) => {
                let mut out = vec![false; items.len()];
                for index in chosen {
                    out[index] = true;
                }
                Some(out)
            }
            _ => None,
        })
    }

    fn input(&mut self, prompt: &str, default: &str) -> Result<Option<String>, UiError> {
        run_input(&mut self.guard("a prompt")?, prompt, default)
    }

    fn confirm(&mut self, prompt: &str, default: bool) -> Result<bool, UiError> {
        run_confirm(&mut self.guard("a prompt")?, prompt, default)
    }
}

impl Ui {
    /// The pickers and prompts. Borrowing the Ui keeps them from
    /// interleaving with any other output.
    ///
    /// Flushes buffered stdout first (best effort; a real failure resurfaces
    /// from the next [`Ui::flush`]), so earlier output cannot land after the
    /// picker.
    pub fn interact(&mut self) -> &mut dyn Interaction {
        self.flush_for_stderr();
        &mut *self.interact
    }
}

/// Waits up to [`KEY_WAIT`] for a key the picker understands; `None` on a
/// timeout or an ignored key, so the caller can re-check the size.
fn next_key(term: &mut impl Term) -> Result<Option<PickerKey>, UiError> {
    Ok(term
        .read_key(KEY_WAIT)?
        .and_then(|event| PickerKey::from_event(&event)))
}

/// Draws `frame` over the `last` one unless nothing changed.
fn redraw(
    term: &mut impl Write,
    last: &mut Vec<String>,
    frame: Vec<String>,
    suffix: &str,
) -> Result<(), UiError> {
    if *last != frame {
        draw(term, last.len(), &frame, suffix)?;
        *last = frame;
    }
    Ok(())
}

/// Usable width: one short of the terminal so a full line never wraps.
fn width(term: &impl Term) -> Result<u16, UiError> {
    Ok(term.size()?.0.saturating_sub(1).max(1))
}

/// Draws the picker until it finishes, then erases it.
fn run_picker(
    term: &mut impl Term,
    prompt: &str,
    state: &mut PickerState<'_>,
) -> Result<PickerStep, UiError> {
    let mut last = Vec::new();
    loop {
        let (w, h) = term.size()?;
        let w = w.saturating_sub(1).max(1);
        let rows = h.saturating_sub(1).clamp(1, MAX_PICKER_ROWS);
        let mut frame = vec![clip(prompt, usize::from(w))];
        frame.extend(state.lines(w, rows));
        redraw(term, &mut last, frame, "")?;
        let drawn = last.len();
        let Some(key) = next_key(term)? else {
            continue;
        };
        match state.apply(key) {
            PickerStep::Continue => {}
            PickerStep::Interrupted => {
                draw(term, drawn, &[], "")?;
                return Err(UiError::Interrupted);
            }
            step => {
                draw(term, drawn, &[], "")?;
                return Ok(step);
            }
        }
    }
}

/// Reads a line, leaving the answered prompt in scrollback.
fn run_input(term: &mut impl Term, prompt: &str, default: &str) -> Result<Option<String>, UiError> {
    let mut input = LineInput::new(prompt, default);
    let mut last = Vec::new();
    loop {
        redraw(term, &mut last, vec![input.line(width(term)?)], SHOW_CURSOR)?;
        let Some(key) = next_key(term)? else {
            continue;
        };
        match input.apply(key) {
            Step::Continue => {}
            Step::Done(answer) => {
                finish(term)?;
                return Ok(answer);
            }
            Step::Interrupted => {
                finish(term)?;
                return Err(UiError::Interrupted);
            }
        }
    }
}

/// Asks yes/no, leaving the question and answer in scrollback.
fn run_confirm(term: &mut impl Term, prompt: &str, default: bool) -> Result<bool, UiError> {
    let confirm = Confirm::new(prompt, default);
    let mut last = Vec::new();
    loop {
        redraw(
            term,
            &mut last,
            vec![confirm.line(None, width(term)?)],
            SHOW_CURSOR,
        )?;
        let Some(key) = next_key(term)? else {
            continue;
        };
        match confirm.apply(key) {
            Step::Continue => {}
            Step::Done(answer) => {
                draw(term, 1, &[confirm.line(Some(answer), width(term)?)], "")?;
                finish(term)?;
                return Ok(answer);
            }
            Step::Interrupted => {
                finish(term)?;
                return Err(UiError::Interrupted);
            }
        }
    }
}

/// Moves past the prompt line.
fn finish(term: &mut impl Term) -> Result<(), UiError> {
    term.write_all(b"\r\n")
        .and_then(|()| term.flush())
        .map_err(|source| UiError::Io { source })
}

/// Replaces the `prev` lines drawn last (the cursor sits on the last of
/// them) with `lines`, then appends `suffix`.
fn draw(term: &mut impl Write, prev: usize, lines: &[String], suffix: &str) -> Result<(), UiError> {
    term.write_all(frame_text(prev, lines, suffix).as_bytes())
        .and_then(|()| term.flush())
        .map_err(|source| UiError::Io { source })
}

/// The bytes for [`draw`]. Raw mode needs `\r\n` between lines.
fn frame_text(prev: usize, lines: &[String], suffix: &str) -> String {
    let mut text = String::from("\r");
    if prev > 1 {
        let _ = write!(text, "\x1b[{}A", prev - 1);
    }
    text.push_str("\x1b[J");
    text.push_str(&lines.join("\r\n"));
    text.push_str(suffix);
    text
}
