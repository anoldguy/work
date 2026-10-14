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
            // Centred under the logo's own width, as in the spec.
            let x = logo_x.saturating_add(to_u16(footer_pad(line)));
            frame.put_str(x, y, line, Some(grey), None);
        }
    }
}

/// Fully saturated colour for step `step` of `PERIOD_ROWS` around the wheel.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "bounded colour math: values are clamped to 0..=255 before narrowing"
)]
fn hue_to_rgb(step: u128) -> Rgb {
    let sector = (step as f64 / PERIOD_ROWS as f64) * 6.0;
    let mid = 1.0 - (sector % 2.0 - 1.0).abs();
    let (red, green, blue) = match sector as u8 {
        0 => (1.0, mid, 0.0),
        1 => (mid, 1.0, 0.0),
        2 => (0.0, 1.0, mid),
        3 => (0.0, mid, 1.0),
        4 => (mid, 0.0, 1.0),
        _ => (1.0, 0.0, mid),
    };
    let ch = |v: f64| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    Rgb {
        r: ch(red),
        g: ch(green),
        b: ch(blue),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::Classic;
    use crate::ui::splash::{Frame, Rgb, Style};

    fn render_sized(ms: u64, w: u16, h: u16) -> Frame {
        let mut run = Classic.start(1, w, h);
        let mut f = Frame::new(w, h);
        run.render(Duration::from_millis(ms), &mut f);
        f
    }

    fn render_at(ms: u64) -> Frame {
        render_sized(ms, 50, 14)
    }

    /// Foreground of the first logo column that is drawn on each logo row.
    fn logo_colors(f: &Frame) -> Vec<Option<Rgb>> {
        (0..14)
            .filter_map(|y| f.get(15, y).filter(|c| c.ch != ' ').map(|c| c.fg))
            .take(6)
            .collect()
    }

    /// Text plus a `#rrggbb` column per row (the fg of the row's first
    /// non-blank cell), so snapshots catch colour regressions.
    fn text_with_colors(f: &Frame) -> String {
        f.to_text()
            .lines()
            .zip(0..f.height())
            .map(|(line, y)| {
                let fg = (0..f.width())
                    .filter_map(|x| f.get(x, y))
                    .find(|c| c.ch != ' ')
                    .and_then(|c| c.fg);
                let tag = fg.map_or_else(String::new, |c| {
                    format!(" #{:02x}{:02x}{:02x}", c.r, c.g, c.b)
                });
                format!("{line}|{tag}")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn snapshot_at_zero() {
        insta::assert_snapshot!(text_with_colors(&render_at(0)));
    }

    #[test]
    fn snapshot_after_one_step() {
        insta::assert_snapshot!(text_with_colors(&render_at(150)));
    }

    #[test]
    fn snapshot_80x24() {
        insta::assert_snapshot!(text_with_colors(&render_sized(300, 80, 24)));
    }

    #[test]
    fn gradient_scrolls_one_row_per_step() {
        let a = logo_colors(&render_at(0));
        let b = logo_colors(&render_at(150));
        assert_eq!(a[..a.len() - 1], b[1..]);
        assert_ne!(a, b);
    }

    #[test]
    fn frame_depends_on_step_not_exact_time() {
        assert_eq!(render_at(10), render_at(149));
        assert_ne!(render_at(0), render_at(150));
    }

    #[test]
    fn tiny_terminal_does_not_panic() {
        let mut run = Classic.start(1, 3, 2);
        let mut f = Frame::new(3, 2);
        run.render(Duration::from_secs(5), &mut f);
    }
}
