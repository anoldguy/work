//! Line prompts (`input`, `confirm`) for `TtyInteraction`.
//!
//! Private to W1.8; nothing outside `ui::interact` depends on this module.
//! Like the picker, each prompt is a pure reducer over decoded
//! [`PickerKey`]s plus a one-line render.

use crate::ui::picker::PickerKey;

/// What the caller does after a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Step<T> {
    /// Keep reading keys and redraw.
    Continue,
    /// The answer (`None` = cancelled, for `input`).
    Done(T),
    /// Ctrl-C.
    Interrupted,
}

/// A line of text with a default offered. Enter on an empty line takes the
/// default; Esc cancels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LineInput<'a> {
    prompt: &'a str,
    default: &'a str,
    buf: String,
}

impl<'a> LineInput<'a> {
    pub(super) fn new(prompt: &'a str, default: &'a str) -> Self {
        Self {
            prompt,
            default,
            buf: String::new(),
        }
    }

    pub(super) fn apply(&mut self, key: PickerKey) -> Step<Option<String>> {
        match key {
            PickerKey::Char(c) => self.buf.push(c),
            PickerKey::Backspace => {
                self.buf.pop();
            }
            PickerKey::Enter if self.buf.is_empty() => {
                return Step::Done(Some(self.default.to_owned()));
            }
            PickerKey::Enter => return Step::Done(Some(std::mem::take(&mut self.buf))),
            PickerKey::Esc => return Step::Done(None),
            PickerKey::CtrlC => return Step::Interrupted,
            PickerKey::Up
            | PickerKey::Down
            | PickerKey::PageUp
            | PickerKey::PageDown
            | PickerKey::Toggle => {}
        }
        Step::Continue
    }

    /// The prompt line, keeping the end (where typing happens) within `w`.
    pub(super) fn line(&self, w: u16) -> String {
        let text = if self.default.is_empty() {
            format!("{}: {}", self.prompt, self.buf)
        } else {
            format!("{} [{}]: {}", self.prompt, self.default, self.buf)
        };
        fit_tail(&text, usize::from(w))
    }
}

/// A yes/no question. `y`/`n` answer at once; Enter takes the default; Esc
/// answers no.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Confirm<'a> {
    prompt: &'a str,
    default: bool,
}

impl<'a> Confirm<'a> {
    pub(super) const fn new(prompt: &'a str, default: bool) -> Self {
        Self { prompt, default }
    }

    pub(super) const fn apply(self, key: PickerKey) -> Step<bool> {
        match key {
            PickerKey::Char('y' | 'Y') => Step::Done(true),
            PickerKey::Char('n' | 'N') | PickerKey::Esc => Step::Done(false),
            PickerKey::Enter => Step::Done(self.default),
            PickerKey::CtrlC => Step::Interrupted,
            _ => Step::Continue,
        }
    }

    /// The question, with the answer once known.
    pub(super) fn line(self, answer: Option<bool>, w: u16) -> String {
        let choices = if self.default { "[Y/n]" } else { "[y/N]" };
        let answer = match answer {
            None => "",
            Some(true) => "yes",
            Some(false) => "no",
        };
        fit_tail(
            &format!("{} {choices} {answer}", self.prompt),
            usize::from(w),
        )
    }
}

/// The last `w` characters of `s`.
fn fit_tail(s: &str, w: usize) -> String {
    let skip = s.chars().count().saturating_sub(w);
    s.chars().skip(skip).collect()
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::{Confirm, LineInput, Step};
    use crate::ui::picker::PickerKey;

    fn typed(input: &mut LineInput<'_>, text: &str) {
        for c in text.chars() {
            assert_eq!(input.apply(PickerKey::Char(c)), Step::Continue);
        }
    }

    #[test]
    fn input_returns_the_typed_text() {
        let mut input = LineInput::new("Work dir", "~/work");
        typed(&mut input, "~/srcx");
        input.apply(PickerKey::Backspace);
        assert_eq!(input.line(80), "Work dir [~/work]: ~/src");
        assert_eq!(
            input.apply(PickerKey::Enter),
            Step::Done(Some("~/src".to_owned()))
        );
    }

    #[test]
    fn input_takes_the_default_on_an_empty_line() {
        let mut input = LineInput::new("Work dir", "~/work");
        assert_eq!(
            input.apply(PickerKey::Enter),
            Step::Done(Some("~/work".to_owned()))
        );
        let mut bare = LineInput::new("Name", "");
        assert_eq!(bare.line(80), "Name: ");
        assert_eq!(
            bare.apply(PickerKey::Enter),
            Step::Done(Some(String::new()))
        );
    }

    #[test]
    fn input_esc_cancels_and_ctrl_c_interrupts() {
        let mut input = LineInput::new("Work dir", "");
        assert_eq!(input.apply(PickerKey::Up), Step::Continue);
        assert_matches!(input.apply(PickerKey::Esc), Step::Done(None));
        assert_matches!(input.apply(PickerKey::CtrlC), Step::Interrupted);
    }

    #[test]
    fn input_line_keeps_the_typing_end_visible() {
        let mut input = LineInput::new("Path", "");
        typed(&mut input, "abcdefgh");
        assert_eq!(input.line(5), "defgh");
    }

    #[test]
    fn confirm_answers() {
        let yes = Confirm::new("Update now?", true);
        assert_eq!(yes.apply(PickerKey::Enter), Step::Done(true));
        assert_eq!(yes.apply(PickerKey::Char('n')), Step::Done(false));
        assert_eq!(yes.apply(PickerKey::Esc), Step::Done(false));
        assert_eq!(yes.apply(PickerKey::Char('x')), Step::Continue);
        assert_matches!(yes.apply(PickerKey::CtrlC), Step::Interrupted);
        let no = Confirm::new("Delete?", false);
        assert_eq!(no.apply(PickerKey::Enter), Step::Done(false));
        assert_eq!(no.apply(PickerKey::Char('Y')), Step::Done(true));
    }

    #[test]
    fn confirm_line_shows_the_default_and_answer() {
        assert_eq!(
            Confirm::new("Update now?", true).line(None, 80),
            "Update now? [Y/n] "
        );
        assert_eq!(
            Confirm::new("Delete?", false).line(Some(true), 80),
            "Delete? [y/N] yes"
        );
    }
}
