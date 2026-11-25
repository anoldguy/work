//! Pickers and prompts on the terminal, all drawn inline on stderr.
//!
//! Every method that waits for input returns [`UiError::Interrupted`] on
//! Ctrl-C.

use std::fmt::Write as _;
use std::io::Write;
use std::time::Duration;

use crate::text::clip;
use crate::ui::picker::{PickerKey, PickerState, PickerStep};
use crate::ui::prompt::{Confirm, LineInput, Step};
use crate::ui::{Screen, TerminalGuard, Ui, UiError};

/// How long one `read_key` waits before the loop re-reads the terminal size
/// (and redraws if it changed).
const KEY_WAIT: Duration = Duration::from_millis(500);

/// The most picker lines (filter line plus items) drawn below the prompt.
const MAX_PICKER_ROWS: u16 = 12;

/// Shows the cursor while a prompt reads a line; the guard hides it.
const SHOW_CURSOR: &str = "\x1b[?25h";

impl Ui {
    const fn require_interactive(&self, what: &'static str) -> Result<(), UiError> {
        if self.interactive {
            Ok(())
        } else {
            Err(UiError::NotInteractive { what })
        }
    }

    fn inline_guard(&mut self, what: &'static str) -> Result<TerminalGuard<'_>, UiError> {
        self.require_interactive(what)?;
        self.flush_for_stderr();
        TerminalGuard::acquire(&mut self.err, Screen::Inline)
    }

    /// Type-to-filter single choice. `Ok(None)` when cancelled with Esc or
    /// given no items.
    ///
    /// # Errors
    ///
    /// [`UiError::NotInteractive`], [`UiError::Interrupted`], or
    /// [`UiError::Io`].
    pub fn pick_one(&mut self, prompt: &str, items: &[&str]) -> Result<Option<usize>, UiError> {
        self.require_interactive("the picker")?;
        if items.is_empty() {
            return Ok(None);
        }
        let mut guard = self.inline_guard("the picker")?;
        let mut state = PickerState::new(items, None);
        let step = run_picker(&mut guard, prompt, &mut state)?;
        Ok(match step {
            PickerStep::Chosen(chosen) => chosen.first().copied(),
            _ => None,
        })
    }

    /// Type-to-filter multi choice with `selected` preselected (same length
    /// as `items`); Tab toggles. `Ok(None)` when cancelled or given no items.
    ///
    /// # Errors
    ///
    /// [`UiError::NotInteractive`], [`UiError::Interrupted`], or
    /// [`UiError::Io`].
    pub fn pick_many(
        &mut self,
        prompt: &str,
        items: &[&str],
        selected: &[bool],
    ) -> Result<Option<Vec<bool>>, UiError> {
        self.require_interactive("the picker")?;
        if items.is_empty() {
            return Ok(None);
        }
        let mut guard = self.inline_guard("the picker")?;
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

    /// A line of text with `default` offered. `Ok(None)` when cancelled.
    ///
    /// # Errors
    ///
    /// [`UiError::NotInteractive`], [`UiError::Interrupted`], or
    /// [`UiError::Io`].
    pub fn input(&mut self, prompt: &str, default: &str) -> Result<Option<String>, UiError> {
        run_input(&mut self.inline_guard("a prompt")?, prompt, default)
    }

    /// A yes/no question; Enter takes `default`.
    ///
    /// # Errors
    ///
    /// [`UiError::NotInteractive`], [`UiError::Interrupted`], or
    /// [`UiError::Io`].
    pub fn confirm(&mut self, prompt: &str, default: bool) -> Result<bool, UiError> {
        run_confirm(&mut self.inline_guard("a prompt")?, prompt, default)
    }
}

/// Waits up to [`KEY_WAIT`] for a key the picker understands; `None` on a
/// timeout or an ignored key, so the caller can re-check the size.
fn next_key(term: &mut TerminalGuard<'_>) -> Result<Option<PickerKey>, UiError> {
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
fn width(term: &TerminalGuard<'_>) -> Result<u16, UiError> {
    Ok(term.size()?.0.saturating_sub(1).max(1))
}

/// Draws the picker until it finishes, then erases it.
fn run_picker(
    term: &mut TerminalGuard<'_>,
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
fn run_input(
    term: &mut TerminalGuard<'_>,
    prompt: &str,
    default: &str,
) -> Result<Option<String>, UiError> {
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
fn run_confirm(term: &mut TerminalGuard<'_>, prompt: &str, default: bool) -> Result<bool, UiError> {
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
fn finish(term: &mut TerminalGuard<'_>) -> Result<(), UiError> {
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
