//! The `classic` style: the logo with a rainbow gradient that scrolls one row per 150 ms.

use std::time::Duration;

use crate::ui::splash::frame::{Frame, Rgb, center, to_u16};
use crate::ui::splash::logo::{BLOCK_HEIGHT, LOGO, WIDTH, footer, footer_pad};
use crate::ui::splash::{Style, StyleRun};

/// Milliseconds the gradient takes to scroll one row.
const STEP_MS: u128 = 150;
/// Rows after which the hue wheel repeats.
const PERIOD_ROWS: u128 = 12;

/// The `classic` style.
#[derive(Debug)]
pub struct Classic;

impl Style for Classic {
    fn name(&self) -> &'static str {
        "classic"
    }

    fn start(&self, seed: u64, w: u16, h: u16) -> Box<dyn StyleRun> {
        let _ = (seed, w, h);
        Box::new(ClassicRun)
    }
}

/// A running `classic` animation (stateless: everything derives from `t`).
#[derive(Debug)]
struct ClassicRun;

impl StyleRun for ClassicRun {
    fn render(&mut self, t: Duration, frame: &mut Frame) {
        frame.clear();
        let phase = (t.as_millis() / STEP_MS) % PERIOD_ROWS;
        let footer = footer(env!("CARGO_PKG_VERSION"));
        let top = center(frame.height(), BLOCK_HEIGHT);
        let logo_x = center(frame.width(), WIDTH);

        for (i, line) in LOGO.iter().enumerate() {
            let y = top.saturating_add(to_u16(i));
            // Colours move one row per step: logo row `i` shows the hue
            // that row `i - phase` showed at t=0.
            let row = u128::try_from(i).unwrap_or(0);
            let hue = (row + PERIOD_ROWS - phase) % PERIOD_ROWS;
            frame.put_str(logo_x, y, line, Some(hue_to_rgb(hue)), None);
        }

        let grey = Rgb {
            r: 200,
            g: 200,
            b: 200,
        };
        for (i, line) in footer.iter().enumerate() {
            let y = top.saturating_add(to_u16(LOGO.len() + 1 + i));
            // Centred under the logo's own width, not the frame's.
            let x = logo_x.saturating_add(to_u16(footer_pad(line)));
            frame.put_str(x, y, line, Some(grey), None);
        }
    }
}

/// Fully saturated colour for step `step` of `PERIOD_ROWS` around the wheel.
#[expect(
    clippy::cast_precision_loss,
    reason = "step is below PERIOD_ROWS (12), exactly representable as f64"
)]
fn hue_to_rgb(step: u128) -> Rgb {
    Rgb::from_hue(step as f64 / PERIOD_ROWS as f64, 255.0, 0.0)
}
