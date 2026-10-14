//! [`TtyInteraction`]: pickers and prompts on the real terminal, and
//! `Ui::interact`.

use std::fmt::Write as _;
use std::io::{self, IsTerminal, Write};
use std::time::Duration;

use crossterm::event::KeyEvent;

use crate::env::EnvSnapshot;
use crate::ui::picker::{PickerKey, PickerState, PickerStep};
use crate::ui::prompt::{Confirm, LineInput, Step};
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
#[derive(Debug)]
pub struct TtyInteraction {
    err: io::Stderr,
    interactive: bool,
}

impl TtyInteraction {
    /// Checks whether stdin and stderr are terminals.
    #[must_use]
    pub fn new() -> Self {
        let err = io::stderr();
        let interactive = io::stdin().is_terminal() && err.is_terminal();
        Self { err, interactive }
    }

    /// Like [`TtyInteraction::new`], but a `TERM=dumb` terminal, which cannot
    /// take raw-mode redraws, counts as not interactive (as `Ui::new` treats
    /// it as `Plain`).
    #[must_use]
    pub fn for_env(env: &EnvSnapshot) -> Self {
        let mut tty = Self::new();
        tty.interactive &= can_redraw(env.term.as_deref());
        tty
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
        TerminalGuard::acquire(&mut self.err, Screen::Inline)
    }
}

impl Default for TtyInteraction {
    fn default() -> Self {
        Self::new()
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

/// Whether a terminal of type `term` can redraw in place.
fn can_redraw(term: Option<&str>) -> bool {
    term != Some("dumb")
}

/// The terminal as the interaction loops see it, so they run against a fake
/// in tests.
trait Term: Write {
    fn read_key(&mut self, timeout: Duration) -> Result<Option<KeyEvent>, UiError>;
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
        let mut frame = vec![prompt.chars().take(usize::from(w)).collect::<String>()];
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

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::collections::VecDeque;
    use std::io::{self, Write};
    use std::time::Duration;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::{Term, TtyInteraction, can_redraw, frame_text, run_confirm, run_input, run_picker};
    use crate::env::EnvSnapshot;
    use crate::testing::{Answer, ScriptedInteraction, SharedBuf};
    use crate::ui::picker::{PickerState, PickerStep};
    use crate::ui::{Interaction, Ui, UiError};

    /// A scripted terminal: replays keys (with a timeout first, to exercise
    /// the wait loop) and records what was drawn. On a timeout it applies
    /// `resize`, if set, as if the window had changed meanwhile.
    struct FakeTerm {
        keys: VecDeque<Option<KeyEvent>>,
        out: Vec<u8>,
        size: (u16, u16),
        resize: Option<(u16, u16)>,
    }

    impl FakeTerm {
        fn new(keys: &[KeyCode]) -> Self {
            let mut queue = VecDeque::from([None]);
            queue.extend(
                keys.iter()
                    .map(|&code| Some(KeyEvent::new(code, KeyModifiers::NONE))),
            );
            Self {
                keys: queue,
                out: Vec::new(),
                size: (40, 10),
                resize: None,
            }
        }

        fn ctrl(mut self, c: char) -> Self {
            self.keys
                .push_back(Some(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)));
            self
        }

        fn output(&self) -> String {
            String::from_utf8_lossy(&self.out).into_owned()
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
        fn read_key(&mut self, _: Duration) -> Result<Option<KeyEvent>, UiError> {
            let key = self.keys.pop_front().expect("the script ran out of keys");
            if key.is_none()
                && let Some(size) = self.resize.take()
            {
                self.size = size;
            }
            Ok(key)
        }
        fn size(&self) -> Result<(u16, u16), UiError> {
            Ok(self.size)
        }
    }

    fn chars(text: &str) -> Vec<KeyCode> {
        text.chars().map(KeyCode::Char).collect()
    }

    const ITEMS: &[&str] = &["acme/widgets", "acme/gadgets", "other/tools"];

    #[test]
    fn picker_filters_chooses_and_erases_its_frame() {
        let mut keys = chars("gad");
        keys.push(KeyCode::Enter);
        let mut term = FakeTerm::new(&keys);
        let mut state = PickerState::new(ITEMS, None);
        let step = run_picker(&mut term, "Pick a repo", &mut state).expect("picker");
        assert_eq!(step, PickerStep::Chosen(vec![1]));
        let out = term.output();
        assert!(out.starts_with("\r\x1b[JPick a repo\r\n>   (3/3)\r\n> acme/widgets"));
        assert!(out.contains("> gad  (1/3)\r\n> acme/gadgets"));
        assert!(out.ends_with("\r\x1b[2A\x1b[J"), "{out:?}");
    }

    #[test]
    fn picker_esc_cancels_and_ctrl_c_interrupts() {
        let mut term = FakeTerm::new(&[KeyCode::Esc]);
        let mut state = PickerState::new(ITEMS, None);
        assert_matches!(
            run_picker(&mut term, "Pick", &mut state),
            Ok(PickerStep::Cancelled)
        );
        let mut term = FakeTerm::new(&[]).ctrl('c');
        assert_matches!(
            run_picker(&mut term, "Pick", &mut state),
            Err(UiError::Interrupted)
        );
    }

    #[test]
    fn picker_redraws_after_a_resize_without_waiting_for_a_key() {
        let mut term = FakeTerm::new(&[KeyCode::Esc]);
        term.resize = Some((8, 10));
        let mut state = PickerState::new(ITEMS, None);
        run_picker(&mut term, "Pick a repo", &mut state).expect("picker");
        let out = term.output();
        assert!(out.starts_with("\r\x1b[JPick a repo\r\n"), "{out:?}");
        assert!(out.contains("\r\x1b[4A\x1b[JPick a \r\n"), "{out:?}");
    }

    #[test]
    fn picker_does_not_redraw_an_unchanged_frame_on_a_timeout() {
        let mut term = FakeTerm::new(&[KeyCode::Esc]);
        let mut state = PickerState::new(ITEMS, None);
        run_picker(&mut term, "Pick a repo", &mut state).expect("picker");
        assert_eq!(term.output().matches("Pick a repo").count(), 1);
    }

    #[test]
    fn a_dumb_terminal_is_not_interactive() {
        let env = EnvSnapshot {
            term: Some("dumb".to_owned()),
            ..EnvSnapshot::default()
        };
        let mut tty = TtyInteraction::for_env(&env);
        assert!(!tty.is_interactive());
        assert_matches!(
            tty.pick_one("Pick", ITEMS),
            Err(UiError::NotInteractive { .. })
        );
        assert!(!can_redraw(Some("dumb")));
        assert!(can_redraw(Some("xterm-256color")));
        assert!(can_redraw(None));
    }

    #[test]
    fn an_empty_item_list_is_cancelled_at_once() {
        let mut tty = TtyInteraction {
            err: io::stderr(),
            interactive: true,
        };
        assert_matches!(tty.pick_one("Pick", &[]), Ok(None));
        assert_matches!(tty.pick_many("Pick", &[], &[]), Ok(None));
    }

    #[test]
    fn picker_lines_fit_inside_the_terminal() {
        let mut term = FakeTerm::new(&[KeyCode::Esc]);
        term.size = (6, 3);
        let mut state = PickerState::new(ITEMS, None);
        run_picker(&mut term, "Pick a repo", &mut state).expect("picker");
        assert!(term.output().starts_with("\r\x1b[JPick \r\n>   (\r\n> acm"));
    }

    #[test]
    fn input_reads_a_line_and_leaves_it_in_scrollback() {
        let mut keys = chars("~/src");
        keys.push(KeyCode::Enter);
        let mut term = FakeTerm::new(&keys);
        assert_eq!(
            run_input(&mut term, "Work dir", "~/work").expect("input"),
            Some("~/src".to_owned())
        );
        assert!(
            term.output()
                .ends_with("Work dir [~/work]: ~/src\x1b[?25h\r\n")
        );
    }

    #[test]
    fn input_cancel_and_interrupt() {
        let mut term = FakeTerm::new(&[KeyCode::Esc]);
        assert_matches!(run_input(&mut term, "Name", ""), Ok(None));
        let mut term = FakeTerm::new(&[]).ctrl('c');
        assert_matches!(run_input(&mut term, "Name", ""), Err(UiError::Interrupted));
    }

    #[test]
    fn confirm_shows_the_answer() {
        let mut term = FakeTerm::new(&[KeyCode::Char('x'), KeyCode::Enter]);
        assert!(run_confirm(&mut term, "Update now?", true).expect("confirm"));
        assert!(term.output().ends_with("\r\x1b[JUpdate now? [Y/n] yes\r\n"));
        let mut term = FakeTerm::new(&[]).ctrl('c');
        assert_matches!(
            run_confirm(&mut term, "Update now?", true),
            Err(UiError::Interrupted)
        );
    }

    #[test]
    fn frame_text_moves_back_over_the_previous_frame() {
        let lines = ["a".to_owned(), "b".to_owned()];
        assert_eq!(frame_text(0, &lines, ""), "\r\x1b[Ja\r\nb");
        assert_eq!(frame_text(1, &lines, ""), "\r\x1b[Ja\r\nb");
        assert_eq!(frame_text(3, &[], "!"), "\r\x1b[2A\x1b[J!");
    }

    #[test]
    fn without_a_terminal_every_method_is_not_interactive() {
        let mut tty = TtyInteraction {
            err: io::stderr(),
            interactive: false,
        };
        assert!(!tty.is_interactive());
        assert_matches!(
            tty.pick_one("Pick", ITEMS),
            Err(UiError::NotInteractive { .. })
        );
        assert_matches!(
            tty.pick_many("Pick", ITEMS, &[false; 3]),
            Err(UiError::NotInteractive { .. })
        );
        assert_matches!(tty.input("Name", ""), Err(UiError::NotInteractive { .. }));
        assert_matches!(
            tty.confirm("Ok?", true),
            Err(UiError::NotInteractive { .. })
        );
    }

    #[test]
    fn interact_flushes_pending_stdout_first() {
        let (out, err) = (SharedBuf::new(), SharedBuf::new());
        let mut ui = Ui::for_test(
            Box::new(out.clone()),
            Box::new(err),
            Box::new(ScriptedInteraction::new(true, vec![Answer::Pick(Some(0))])),
        );
        ui.stdout_line("before").expect("write");
        assert_eq!(out.contents(), "");
        assert_matches!(ui.interact().pick_one("Pick", ITEMS), Ok(Some(0)));
        assert_eq!(out.contents(), "before\n");
    }
}
