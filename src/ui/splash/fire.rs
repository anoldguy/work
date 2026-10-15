//! The `fire` style: a demoscene heat-map fire below the logo.
//!
//! The fire is simulated on a grid with twice the screen's vertical
//! resolution and drawn with half blocks (`▀`: top half in the foreground,
//! bottom half in the background). The simulation advances in fixed steps of
//! wall-clock time, so a dropped frame runs several steps at once.

use std::num::NonZeroU32;
use std::time::Duration;

use crate::ui::splash::frame::{Cell, Frame, Rgb, center, to_u16};
use crate::ui::splash::logo::{BLOCK_HEIGHT, LOGO, WIDTH, footer, footer_pad};
use crate::ui::splash::rng::Rng;
use crate::ui::splash::{Style, StyleRun};

/// Milliseconds per simulation step.
const STEP_MS: u128 = 33;
/// The most steps one render catches up on; a longer stall just skips ahead.
const MAX_CATCH_UP: u128 = 90;
/// Heat below this draws nothing.
const MIN_VISIBLE: u16 = 12;
/// Fresh fuel is `150 + 0..FUEL_RANGE`, so at most 255.
const FUEL_RANGE: NonZeroU32 = NonZeroU32::new(106).expect("106 is non-zero");

/// The `fire` style.
#[derive(Debug)]
pub struct Fire;

impl Style for Fire {
    fn name(&self) -> &'static str {
        "fire"
    }

    fn start(&self, seed: u64, w: u16, h: u16) -> Box<dyn StyleRun> {
        Box::new(FireRun {
            rng: Rng::new(seed),
            width: w,
            rows: usize::from(h) * 2,
            heat: vec![0; usize::from(w) * usize::from(h) * 2],
            steps_done: 0,
        })
    }
}

/// A running `fire` animation.
#[derive(Debug)]
struct FireRun {
    rng: Rng,
    width: u16,
    /// Heat rows: twice the screen height.
    rows: usize,
    /// Row-major heat, `0..=255`; row 0 is the top.
    heat: Vec<u8>,
    steps_done: u128,
}

impl FireRun {
    /// Heat at (`x`, `y`), 0 outside the grid.
    fn at(&self, x: usize, y: usize) -> u16 {
        if x >= usize::from(self.width) || y >= self.rows {
            return 0;
        }
        self.heat
            .get(y * usize::from(self.width) + x)
            .map_or(0, |h| u16::from(*h))
    }

    /// Advances the simulation by one step.
    fn step(&mut self) {
        let w = usize::from(self.width);
        if w == 0 || self.rows == 0 {
            return;
        }
        // Flames reach about two fifths of the way up the screen.
        let flame_rows = u32::try_from(self.rows * 2 / 5).unwrap_or(u32::MAX).max(1);
        let max_cool =
            NonZeroU32::new((2 * 255 / flame_rows).clamp(1, 255) + 1).unwrap_or(NonZeroU32::MIN);

        // Fresh fuel along the bottom.
        let last = self.rows - 1;
        for x in 0..w {
            let fuel = 150 + self.rng.below(FUEL_RANGE);
            if let Some(h) = self.heat.get_mut(last * w + x) {
                *h = u8::try_from(fuel).unwrap_or(255);
            }
        }
        // Rows are read from below the one being written, which has not
        // been updated this step yet, so one top-to-bottom pass is enough.
        for y in 0..last {
            for x in 0..w {
                let sum = self.at(x.saturating_sub(1), y + 1)
                    + self.at(x, y + 1)
                    + self.at(x + 1, y + 1)
                    + self.at(x, y + 2);
                let cool = u16::try_from(self.rng.below(max_cool)).unwrap_or(0);
                if let Some(h) = self.heat.get_mut(y * w + x) {
                    *h = u8::try_from((sum / 4).saturating_sub(cool)).unwrap_or(255);
                }
            }
        }
    }

    /// Mean heat of the three columns around `x` at 70% of the grid height.
    fn glow(&self, x: usize) -> u16 {
        let y = self.rows * 7 / 10;
        (self.at(x.saturating_sub(1), y) + self.at(x, y) + self.at(x + 1, y)) / 3
    }

    /// Draws the logo and footer above the flames, lit from below: rows
    /// nearer the fire are brighter and the light flickers with the flames.
    fn draw_logo(&self, frame: &mut Frame) {
        let footer = footer(env!("CARGO_PKG_VERSION"));
        let h = usize::from(frame.height());
        // Centre the block in the part of the screen the flames leave free.
        let free = to_u16(h - h * 2 / 5);
        let top = usize::from(center(free, BLOCK_HEIGHT));
        let logo_x = usize::from(center(frame.width(), WIDTH));

        // Footer lines are centred under the logo's own width.
        let lines = LOGO.iter().enumerate().map(|(i, l)| (i, 0, *l)).chain(
            footer
                .iter()
                .enumerate()
                .map(|(i, l)| (LOGO.len() + 1 + i, footer_pad(l), l.as_str())),
        );
        for (row, pad, line) in lines {
            let y = to_u16(top + row);
            for (dx, ch) in line.chars().enumerate() {
                if ch == ' ' {
                    continue;
                }
                let x = logo_x + pad + dx;
                let lit = lit_heat(row, BLOCK_HEIGHT, self.glow(x));
                frame.set(
                    to_u16(x),
                    y,
                    Cell {
                        ch,
                        fg: Some(palette(lit)),
                        bg: None,
                    },
                );
            }
        }
    }
}

impl StyleRun for FireRun {
    fn render(&mut self, t: Duration, frame: &mut Frame) {
        let target = t.as_millis() / STEP_MS;
        let behind = target.saturating_sub(self.steps_done);
        for _ in 0..behind.min(MAX_CATCH_UP) {
            self.step();
        }
        self.steps_done = self.steps_done.max(target);

        frame.clear();
        for y in 0..frame.height() {
            for x in 0..frame.width() {
                let top = self.at(usize::from(x), usize::from(y) * 2);
                let bottom = self.at(usize::from(x), usize::from(y) * 2 + 1);
                if top.max(bottom) < MIN_VISIBLE {
                    continue;
                }
                frame.set(
                    x,
                    y,
                    Cell {
                        ch: '▀',
                        fg: Some(palette(top)),
                        bg: Some(palette(bottom)),
                    },
                );
            }
        }
        self.draw_logo(frame);
    }
}

/// Heat a logo cell on `row` of `total` receives: more toward the bottom,
/// plus a flicker from the flames' `glow`.
fn lit_heat(row: usize, total: usize, glow: u16) -> u16 {
    let rows = to_u16(total.saturating_sub(1)).max(1);
    let row = to_u16(row).min(rows);
    // 110..=210 from top to bottom, then up to +51 from the glow.
    let base = 110 + 100 * row / rows;
    (base + glow / 5).min(255)
}

/// Black through red and orange to yellow and white for heat `0..=255`.
fn palette(heat: u16) -> Rgb {
    let h = heat.min(255);
    let ch = |v: u16| u8::try_from(v.min(255)).unwrap_or(255);
    match h {
        0..85 => Rgb {
            r: ch(h * 3),
            g: 0,
            b: 0,
        },
        85..170 => Rgb {
            r: 255,
            g: ch((h - 85) * 3),
            b: 0,
        },
        _ => Rgb {
            r: 255,
            g: 255,
            b: ch((h - 170) * 3),
        },
    }
}
