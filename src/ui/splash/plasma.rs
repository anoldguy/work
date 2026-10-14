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

/// Fully saturated colour at `hue` in `[0, 1)` around the wheel.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "bounded colour math: values are clamped to 0..=255 before narrowing"
)]
fn hue_to_rgb(hue: f64) -> Rgb {
    let sector = hue.rem_euclid(1.0) * 6.0;
    let mid = 1.0 - (sector % 2.0 - 1.0).abs();
    let (red, green, blue) = match sector as u8 {
        0 => (1.0, mid, 0.0),
        1 => (mid, 1.0, 0.0),
        2 => (0.0, 1.0, mid),
        3 => (0.0, mid, 1.0),
        4 => (mid, 0.0, 1.0),
        _ => (1.0, 0.0, mid),
    };
    // Slightly dimmed so the white logo stays readable.
    let ch = |v: f64| (v * 200.0 + 30.0).round().clamp(0.0, 255.0) as u8;
    Rgb {
        r: ch(red),
        g: ch(green),
        b: ch(blue),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{BOX, Plasma};
    use crate::ui::splash::{Frame, Style};

    fn render(seed: u64, ms: u64, w: u16, h: u16) -> Frame {
        let mut run = Plasma.start(seed, w, h);
        let mut f = Frame::new(w, h);
        run.render(Duration::from_millis(ms), &mut f);
        f
    }

    // 80×24 so the field shows around the logo box; one colour code per
    // field cell (see `Frame::to_color_map`), the box as `0`.

    #[test]
    fn snapshot_at_zero() {
        insta::assert_snapshot!(render(1, 0, 80, 24).to_color_map());
    }

    #[test]
    fn snapshot_after_two_seconds() {
        insta::assert_snapshot!(render(1, 2000, 80, 24).to_color_map());
    }

    #[test]
    fn snapshot_other_seed_and_time() {
        insta::assert_snapshot!(render(99, 7300, 80, 24).to_color_map());
    }

    #[test]
    fn the_field_outside_the_box_is_coloured_and_moves() {
        let corner = |seed, ms| render(seed, ms, 80, 24).get(0, 0).and_then(|c| c.bg);
        let at_rest = corner(1, 0);
        assert!(at_rest.is_some_and(|c| c != BOX), "{at_rest:?}");
        assert_ne!(at_rest, corner(1, 2000));
        assert_ne!(at_rest, corner(2, 0));
    }

    #[test]
    fn logo_is_drawn_over_the_field() {
        let f = render(1, 500, 50, 14);
        assert!(f.to_text().contains("██╗"));
        assert!(f.to_text().contains("github.com/anoldguy/work"));
    }

    #[test]
    fn same_seed_and_time_gives_same_frame() {
        assert_eq!(render(5, 1234, 80, 24), render(5, 1234, 80, 24));
    }

    #[test]
    fn seed_and_time_change_the_frame() {
        assert_ne!(render(5, 1234, 80, 24), render(6, 1234, 80, 24));
        assert_ne!(render(5, 0, 80, 24), render(5, 3000, 80, 24));
    }

    #[test]
    fn tiny_and_zero_frames_do_not_panic() {
        for (w, h) in [(0, 0), (0, 5), (5, 0), (1, 1), (3, 2)] {
            render(1, 5000, w, h);
        }
    }
}
