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
const PROMPT: &str = "press any key to exit";
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
