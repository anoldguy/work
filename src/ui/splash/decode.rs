//! The `decode` style: scrambled glyphs that lock in column by column, then a diagonal shimmer.

use std::num::NonZeroU32;
use std::time::Duration;

use crate::ui::splash::frame::{Cell, Frame, Rgb, center, to_u16};
use crate::ui::splash::logo::{BLOCK_HEIGHT, LOGO, WIDTH, footer, footer_pad};
use crate::ui::splash::rng::Rng;
use crate::ui::splash::{Style, StyleRun};

/// Milliseconds over which the columns lock, left to right.
const LOCK_MS: u128 = 1600;
/// Milliseconds a scrambled glyph stays before changing.
const SCRAMBLE_MS: u128 = 60;
/// Milliseconds between shimmer sweeps (measured from the end of the lock).
const SHIMMER_PERIOD_MS: u128 = 3000;
/// Milliseconds one shimmer sweep takes.
const SHIMMER_SWEEP_MS: u128 = 700;
/// Milliseconds per typed footer character.
const TYPE_MS: u128 = 30;
/// Half-width of the shimmer band, in diagonal units.
const SHIMMER_HALF_WIDTH: i64 = 2;
/// The most extra milliseconds (exclusive) a column may lock late.
const JITTER_MS: NonZeroU32 = NonZeroU32::new(120).expect("120 is non-zero");

/// Glyphs a column shows before it locks.
const GLYPHS: &[char] = &[
    '█', '▓', '▒', '░', '╔', '╗', '╚', '╝', '║', '═', '#', '%', '@', '0', '1', '$', '&', '/',
];

const LOCKED: Rgb = Rgb {
    r: 60,
    g: 220,
    b: 120,
};
const SCRAMBLED: Rgb = Rgb {
    r: 20,
    g: 110,
    b: 60,
};
const SHIMMER: Rgb = Rgb {
    r: 255,
    g: 255,
    b: 255,
};
const GREY: Rgb = Rgb {
    r: 200,
    g: 200,
    b: 200,
};

/// The `decode` style.
#[derive(Debug)]
pub struct Decode;

impl Style for Decode {
    fn name(&self) -> &'static str {
        "decode"
    }

    fn start(&self, seed: u64, w: u16, h: u16) -> Box<dyn StyleRun> {
        let _ = (w, h);
        // Per-column lock jitter drawn once from the seed.
        let mut rng = Rng::new(seed);
        let jitter = (0..WIDTH)
            .map(|_| u128::from(rng.below(JITTER_MS)))
            .collect();
        Box::new(DecodeRun { seed, jitter })
    }
}

/// A running `decode` animation (stateless: everything derives from `t`).
#[derive(Debug)]
struct DecodeRun {
    seed: u64,
    /// Extra milliseconds before each logo column locks.
    jitter: Vec<u128>,
}

impl DecodeRun {
    /// When logo column `col` of `cols` locks, in milliseconds.
    fn lock_ms(&self, col: usize, cols: usize) -> u128 {
        let base = LOCK_MS.saturating_sub(u128::from(JITTER_MS.get()))
            * u128::try_from(col).unwrap_or(0)
            / u128::try_from(cols.max(1)).unwrap_or(1);
        base + self.jitter.get(col).copied().unwrap_or(0)
    }

    /// The scrambled glyph for a cell at a time tick.
    fn glyph(&self, col: usize, row: usize, tick: u128) -> char {
        let mix = self
            .seed
            .wrapping_add(u64::try_from(col).unwrap_or(0).wrapping_mul(0x9E37_79B9))
            .wrapping_add(u64::try_from(row).unwrap_or(0).wrapping_mul(0x85EB_CA6B))
            .wrapping_add(u64::try_from(tick).unwrap_or(0).wrapping_mul(0xC2B2_AE35));
        let mut rng = Rng::new(mix);
        let n =
            NonZeroU32::new(u32::try_from(GLYPHS.len()).unwrap_or(1)).unwrap_or(NonZeroU32::MIN);
        GLYPHS
            .get(usize::try_from(rng.below(n)).unwrap_or(0))
            .copied()
            .unwrap_or('#')
    }
}

impl StyleRun for DecodeRun {
    fn render(&mut self, t: Duration, frame: &mut Frame) {
        frame.clear();
        let ms = t.as_millis();
        let footer = footer(env!("CARGO_PKG_VERSION"));
        let top = center(frame.height(), BLOCK_HEIGHT);
        let cols = WIDTH;
        let left = center(frame.width(), cols);

        let tick = ms / SCRAMBLE_MS;
        let after_lock = ms.checked_sub(LOCK_MS);
        // Shimmer position along the diagonal `col + 2 * row`, if sweeping.
        let diagonals = i64::try_from(cols + 2 * LOGO.len()).unwrap_or(0);
        let shimmer_at = after_lock.and_then(|a| {
            let in_period = a % SHIMMER_PERIOD_MS;
            (in_period < SHIMMER_SWEEP_MS).then(|| {
                let span = diagonals + 2 * SHIMMER_HALF_WIDTH;
                i64::try_from(in_period).unwrap_or(0) * span
                    / i64::try_from(SHIMMER_SWEEP_MS).unwrap_or(1)
                    - SHIMMER_HALF_WIDTH
            })
        });

        for (row, line) in LOGO.iter().enumerate() {
            let y = top.saturating_add(to_u16(row));
            for (col, ch) in line.chars().enumerate() {
                if ch == ' ' {
                    continue;
                }
                let x = left.saturating_add(to_u16(col));
                let cell = if ms >= self.lock_ms(col, cols) {
                    let diag = i64::try_from(col + 2 * row).unwrap_or(0);
                    let lit = shimmer_at.is_some_and(|p| (diag - p).abs() <= SHIMMER_HALF_WIDTH);
                    Cell {
                        ch,
                        fg: Some(if lit { SHIMMER } else { LOCKED }),
                        bg: None,
                    }
                } else {
                    Cell {
                        ch: self.glyph(col, row, tick),
                        fg: Some(SCRAMBLED),
                        bg: None,
                    }
                };
                frame.set(x, y, cell);
            }
        }

        // The footer types itself in after the lock completes.
        let Some(typed_ms) = after_lock else { return };
        let mut remaining = usize::try_from(typed_ms / TYPE_MS).unwrap_or(usize::MAX);
        for (i, line) in footer.iter().enumerate() {
            let shown: String = line.chars().take(remaining).collect();
            remaining = remaining.saturating_sub(line.chars().count());
            let y = top.saturating_add(to_u16(LOGO.len() + 1 + i));
            let x = left.saturating_add(to_u16(footer_pad(line)));
            frame.put_str(x, y, &shown, Some(GREY), None);
        }
    }
}
