//! The splash event loop: draws a style at 60 fps until a key is pressed.

use std::time::{Duration, Instant};

use crate::ui::splash::renderer::Renderer;
use crate::ui::splash::{Frame, Rgb, Style, StyleRun};
use crate::ui::terminal::Term;
use crate::ui::{Screen, Ui, UiError};

/// One frame at 60 fps.
const FRAME: Duration = Duration::from_micros(16_667);
/// The exit prompt toggles this often.
const BLINK: Duration = Duration::from_millis(500);
/// The exit prompt text.
const PROMPT: &str = "press any key to exit";
/// The exit prompt colour.
const GREY: Rgb = Rgb {
    r: 160,
    g: 160,
    b: 160,
};

impl Ui {
    /// Plays `style` (randomness from `seed`) in the alternate screen until
    /// a key is pressed, in colour unless this Ui is colourless.
    ///
    /// # Errors
    ///
    /// [`UiError::NotInteractive`] in `Plain` mode, or [`UiError::Io`].
    pub fn splash(&mut self, style: &dyn Style, seed: u64) -> Result<(), UiError> {
        let color = self.color();
        let mut guard = self.terminal(Screen::Alternate)?;
        let start = Instant::now();
        run_splash(&mut guard, &|| start.elapsed(), style, seed, color)
    }
}

/// The style run for the current terminal size, with its frame buffer.
type Running = ((u16, u16), Box<dyn StyleRun>, Frame);

/// Renders frames of `style` on `term` until a key is pressed, restarting
/// the style when the terminal is resized. `clock` is the time since the
/// splash started; each frame waits for a key for what is left of its
/// budget, so a slow frame skips ahead rather than catching up.
fn run_splash(
    term: &mut impl Term,
    clock: &impl Fn() -> Duration,
    style: &dyn Style,
    seed: u64,
    color: bool,
) -> Result<(), UiError> {
    let mut renderer = Renderer::new();
    let mut state: Option<Running> = None;
    loop {
        let size = term.size()?;
        let (_, run, frame) = match &mut state {
            Some(s) if s.0 == size => s,
            slot => slot.insert((
                size,
                style.start(seed, size.0, size.1),
                Frame::new(size.0, size.1),
            )),
        };
        let t = clock();
        frame.clear();
        run.render(t, frame);
        if blink_on(t) {
            draw_prompt(frame);
        }
        renderer
            .draw(frame, color, term)
            .map_err(|source| UiError::Io { source })?;
        let spent = clock().saturating_sub(t);
        if term.read_key(FRAME.saturating_sub(spent))?.is_some() {
            return Ok(());
        }
    }
}

/// Whether the blinking prompt is lit at `t`.
fn blink_on(t: Duration) -> bool {
    (t.as_millis() / BLINK.as_millis()).is_multiple_of(2)
}

/// Writes the prompt centred on the bottom row.
fn draw_prompt(frame: &mut Frame) {
    let x = super::frame::center(frame.width(), PROMPT.chars().count());
    let y = frame.height().saturating_sub(1);
    frame.put_str(x, y, PROMPT, Some(GREY), None);
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use std::assert_matches;

    use super::{FRAME, blink_on, run_splash};
    use crate::testing::{ScriptedInteraction, SharedBuf};
    use crate::ui::splash::find_style;
    use crate::ui::terminal::fake::FakeTerm;
    use crate::ui::{Ui, UiError};

    #[test]
    fn a_plain_ui_cannot_play_a_splash() {
        let err = SharedBuf::new();
        let mut ui = Ui::for_test(
            Box::new(SharedBuf::new()),
            Box::new(err.clone()),
            Box::new(ScriptedInteraction::new(false, vec![])),
        );
        let style = find_style("classic").expect("a registered style");
        assert_matches!(ui.splash(style, 1), Err(UiError::NotInteractive { .. }));
        assert_eq!(err.contents(), "");
    }

    /// Runs `style` (seed 1) on a 60×14 `term` whose script times out
    /// `frames - 1` times and then presses a key.
    fn play(name: &str, color: bool, term: FakeTerm, frames: usize) -> FakeTerm {
        let mut term = term
            .timeouts(frames - 1)
            .keys(&[crossterm::event::KeyCode::Enter]);
        let style = find_style(name).expect("a registered style");
        let clock = term.clock();
        run_splash(&mut term, &clock, style, 1, color).expect("splash");
        term
    }

    #[test]
    fn draws_until_a_key_and_then_stops() {
        let term = play("classic", false, FakeTerm::new((60, 14)), 3);
        let clock = term.clock();
        // Two full frame waits passed before the key.
        assert_eq!(clock(), FRAME * 2);
        let text = term.output();
        assert!(text.starts_with("\x1b[0m\x1b[2J"), "{text:?}");
        // Blank cells are skipped, so the prompt's words arrive apart.
        assert!(text.contains("press") && text.contains("exit"), "{text:?}");
    }

    #[test]
    fn later_frames_are_diffs_not_full_redraws() {
        let first = play("decode", true, FakeTerm::new((60, 14)), 1).written();
        let five = play("decode", true, FakeTerm::new((60, 14)), 5).written();
        // Frames 2–5 (67 ms) rescramble the logo once; each full redraw
        // would cost as much as the first frame.
        assert!(five < first * 3, "{five} vs {first}");
        assert!(five > first, "later frames drew nothing: {five} vs {first}");
    }

    #[test]
    fn resize_restarts_with_a_full_redraw() {
        let term = FakeTerm::new((60, 14)).resize_on_timeout((70, 16));
        let text = play("classic", false, term, 2).output();
        assert_eq!(text.matches("\x1b[2J").count(), 2);
    }

    #[test]
    fn prompt_blinks() {
        assert!(blink_on(Duration::ZERO));
        assert!(!blink_on(Duration::from_millis(500)));
        assert!(blink_on(Duration::from_millis(1000)));
    }
}
