//! The `dropin` style: logo columns fall in on a spring; the footer springs up.

use std::num::NonZeroU32;
use std::time::Duration;

use crate::ui::splash::frame::{Cell, Frame, Rgb, center, to_u16};
use crate::ui::splash::logo::{BLOCK_HEIGHT, LOGO, WIDTH, footer, footer_pad};
use crate::ui::splash::rng::Rng;
use crate::ui::splash::{Style, StyleRun};

/// Milliseconds between one column starting to fall and the next.
const STAGGER_MS: f64 = 25.0;
/// Spring natural frequency, rad/s.
const OMEGA: f64 = 11.0;
/// Damping ratio (about 13% overshoot; the rebound is under 2%).
const ZETA: f64 = 0.55;
/// Milliseconds after the last column starts before the footer starts.
const FOOTER_GAP_MS: f64 = 600.0;
/// The most extra start delay (exclusive) per column, in milliseconds.
const JITTER_MS: NonZeroU32 = NonZeroU32::new(15).expect("15 is non-zero");

const BODY: Rgb = Rgb {
    r: 255,
    g: 190,
    b: 70,
};
const GREY: Rgb = Rgb {
    r: 200,
    g: 200,
    b: 200,
};

/// The `dropin` style.
#[derive(Debug)]
pub struct Dropin;

impl Style for Dropin {
    fn name(&self) -> &'static str {
        "dropin"
    }

    fn start(&self, seed: u64, w: u16, h: u16) -> Box<dyn StyleRun> {
        let _ = (w, h);
        let mut rng = Rng::new(seed);
        let jitter = (0..WIDTH)
            .map(|_| f64::from(rng.below(JITTER_MS)))
            .collect();
        Box::new(DropinRun { jitter })
    }
}

/// A running `dropin` animation (stateless: everything derives from `t`).
#[derive(Debug)]
struct DropinRun {
    /// Extra start delay per column in milliseconds, from the seed.
    jitter: Vec<f64>,
}

/// Remaining displacement from rest as a fraction of the start distance
/// after `s` seconds on a damped spring: 1 at `s = 0`, crossing 0 once and
/// overshooting before settling at 0.
fn spring(s: f64) -> f64 {
    if s <= 0.0 {
        return 1.0;
    }
    let wd = OMEGA * (1.0 - ZETA * ZETA).sqrt();
    let decay = (-ZETA * OMEGA * s).exp();
    decay * ((wd * s).cos() + (ZETA * OMEGA / wd) * (wd * s).sin())
}

/// Rounds a row displacement to whole rows.
#[expect(
    clippy::cast_possible_truncation,
    reason = "displacement is clamped to a small range before narrowing"
)]
fn rows(v: f64) -> i32 {
    v.round().clamp(-1.0e6, 1.0e6) as i32
}

/// Writes `cell` at column `x`, row `y` (signed); off-screen rows are skipped.
fn put(frame: &mut Frame, x: u16, y: i32, cell: Cell) {
    if let Ok(y) = u16::try_from(y) {
        frame.set(x, y, cell);
    }
}

impl StyleRun for DropinRun {
    fn render(&mut self, t: Duration, frame: &mut Frame) {
        frame.clear();
        let ms = t.as_secs_f64() * 1000.0;
        let footer = footer(env!("CARGO_PKG_VERSION"));
        let top = center(frame.height(), BLOCK_HEIGHT);
        let left = center(frame.width(), WIDTH);
        let top_i = i32::from(top);
        // Columns start above the screen; the footer starts below it.
        let fall = f64::from(top) + f64::from(to_u16(LOGO.len())) + 1.0;
        let rise = f64::from(frame.height().saturating_sub(top)) + 1.0;

        for (row, line) in LOGO.iter().enumerate() {
            let rest = top_i + i32::try_from(row).unwrap_or(0);
            for (col, ch) in line.chars().enumerate() {
                if ch == ' ' {
                    continue;
                }
                let delay = STAGGER_MS * f64::from(to_u16(col))
                    + self.jitter.get(col).copied().unwrap_or(0.0);
                let s = (ms - delay) / 1000.0;
                let y = rest - rows(fall * spring(s));
                let x = left.saturating_add(to_u16(col));
                let cell = Cell {
                    ch,
                    fg: Some(BODY),
                    bg: None,
                };
                put(frame, x, y, cell);
            }
        }

        // The footer starts a beat after the last column has started.
        let last_delay = STAGGER_MS * f64::from(to_u16(WIDTH));
        let s = (ms - last_delay - FOOTER_GAP_MS) / 1000.0;
        if s <= 0.0 {
            return;
        }
        let shift = rows(rise * spring(s));
        for (i, line) in footer.iter().enumerate() {
            let rest = top_i + i32::try_from(LOGO.len() + 1 + i).unwrap_or(0);
            let x = left.saturating_add(to_u16(footer_pad(line)));
            for (dx, ch) in line.chars().enumerate() {
                let cx = x.saturating_add(to_u16(dx));
                let cell = Cell {
                    ch,
                    fg: Some(GREY),
                    bg: None,
                };
                put(frame, cx, rest + shift, cell);
            }
        }
    }
}
