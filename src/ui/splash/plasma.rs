//! The `plasma` style: a full-screen sine-field plasma behind the logo.

use std::f64::consts::TAU;
use std::time::Duration;

use crate::ui::splash::frame::{Cell, Frame, Rgb, center, to_u16};
use crate::ui::splash::logo::{BLOCK_HEIGHT, LOGO, WIDTH, footer, footer_pad};
use crate::ui::splash::rng::Rng;
use crate::ui::splash::{Style, StyleRun};

/// Seconds for the whole field to cycle once around the hue wheel.
const HUE_PERIOD_SECS: f64 = 12.0;
/// Colour of the box the logo sits in.
const BOX: Rgb = Rgb {
    r: 12,
    g: 12,
    b: 20,
};
/// Colour of the logo and footer text.
const WHITE: Rgb = Rgb {
    r: 255,
    g: 255,
    b: 255,
};

/// The `plasma` style.
#[derive(Debug)]
pub struct Plasma;

impl Style for Plasma {
    fn name(&self) -> &'static str {
        "plasma"
    }

    fn start(&self, seed: u64, w: u16, h: u16) -> Box<dyn StyleRun> {
        let mut rng = Rng::new(seed);
        let phases = [
            rng.unit() * TAU,
            rng.unit() * TAU,
            rng.unit() * TAU,
            rng.unit() * TAU,
        ];
        // Cells are about twice as tall as wide, so the field's y unit is
        // half a row and the screen's middle is at (w / 2, h).
        let middle = (f64::from(w) / 2.0, f64::from(h));
        Box::new(PlasmaRun { phases, middle })
    }
}

/// A running `plasma` animation (stateless apart from the seeded phases).
#[derive(Debug)]
struct PlasmaRun {
    /// Per-wave phase offsets drawn from the seed.
    phases: [f64; 4],
    /// The centre of the radial wave, in field units.
    middle: (f64, f64),
}

impl PlasmaRun {
    /// The hue in `[0, 1)` of cell (`x`, `y`) at `secs` seconds.
    fn hue_at(&self, x: u16, y: u16, secs: f64) -> f64 {
        let (fx, fy) = (f64::from(x), f64::from(y) * 2.0);
        let [p0, p1, p2, p3] = self.phases;
        let (cx, cy) = (fx - self.middle.0, fy - self.middle.1);
        let sum = (fx * 0.11 + secs * 0.9 + p0).sin()
            + (fy * 0.14 - secs * 1.1 + p1).sin()
            + ((fx + fy) * 0.07 + secs * 0.6 + p2).sin()
            + (cx.hypot(cy) * 0.09 - secs * 1.3 + p3).sin();
        // `sum` is in [-4, 4]; one trip across that range is one trip round
        // the wheel, and the whole field drifts with time.
        let hue = sum / 8.0 + 0.5 + secs / HUE_PERIOD_SECS;
        hue.rem_euclid(1.0)
    }
}

impl StyleRun for PlasmaRun {
    fn render(&mut self, t: Duration, frame: &mut Frame) {
        frame.clear();
        let secs = t.as_secs_f64();
        for y in 0..frame.height() {
            for x in 0..frame.width() {
                let bg = Some(hue_to_rgb(self.hue_at(x, y, secs)));
                frame.set(
                    x,
                    y,
                    Cell {
                        ch: ' ',
                        fg: None,
                        bg,
                    },
                );
            }
        }
        draw_logo_box(frame);
    }
}

/// Draws the logo and footer, white on a dark box, centred in `frame`.
fn draw_logo_box(frame: &mut Frame) {
    let footer = footer(env!("CARGO_PKG_VERSION"));
    let top = center(frame.height(), BLOCK_HEIGHT);
    let logo_x = center(frame.width(), WIDTH);

    // Box with one row and two columns of padding around the text.
    let box_x = logo_x.saturating_sub(2);
    let box_w = to_u16(WIDTH).saturating_add(4);
    let blank = " ".repeat(usize::from(box_w));
    for i in 0..to_u16(BLOCK_HEIGHT).saturating_add(2) {
        if let Some(y) = top.saturating_add(i).checked_sub(1) {
            frame.put_str(box_x, y, &blank, None, Some(BOX));
        }
    }

    for (i, line) in LOGO.iter().enumerate() {
        let y = top.saturating_add(to_u16(i));
        frame.put_str(logo_x, y, line, Some(WHITE), Some(BOX));
    }
    for (i, line) in footer.iter().enumerate() {
        let y = top.saturating_add(to_u16(LOGO.len() + 1 + i));
        let x = logo_x.saturating_add(to_u16(footer_pad(line)));
        frame.put_str(x, y, line, Some(WHITE), Some(BOX));
    }
}

/// Colour at `hue` in `[0, 1)` around the wheel, slightly dimmed so the
/// white logo stays readable.
fn hue_to_rgb(hue: f64) -> Rgb {
    Rgb::from_hue(hue, 200.0, 30.0)
}
