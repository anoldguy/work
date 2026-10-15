//! The `about` splash: a registry of animation styles that render into a
//! [`Frame`] buffer.
//!
//! Styles are time-based (a dropped frame skips ahead) and draw randomness
//! only from the seed they are started with, so a frame is fixed by seed,
//! time and size.

pub mod classic;
pub mod decode;
pub mod dropin;
pub mod frame;
pub mod logo;
pub mod plasma;
pub mod renderer;
pub mod rng;
mod run;

pub use frame::{Cell, Frame, Rgb};

use std::time::Duration;

/// An animation style. `Sync` so the registry can be a `static`.
pub trait Style: Sync {
    /// The `--style` name, e.g. `classic`.
    fn name(&self) -> &'static str;

    /// Starts a run for a `w`×`h` terminal with randomness from `seed`.
    fn start(&self, seed: u64, w: u16, h: u16) -> Box<dyn StyleRun>;
}

/// One running animation, holding any state the style keeps between frames.
pub trait StyleRun {
    /// Draws the frame at time `t` since the start into `frame` (which has
    /// the run's size).
    fn render(&mut self, t: Duration, frame: &mut Frame);
}

/// Every style, in registry order.
pub static STYLES: &[&dyn Style] = &[
    &classic::Classic,
    &decode::Decode,
    &plasma::Plasma,
    &dropin::Dropin,
];

/// The style called `name`, if any.
#[must_use]
pub fn find_style(name: &str) -> Option<&'static dyn Style> {
    STYLES.iter().copied().find(|s| s.name() == name)
}

/// Every style name, in registry order.
#[must_use]
pub fn style_names() -> Vec<&'static str> {
    STYLES.iter().map(|s| s.name()).collect()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{Frame, find_style};

    /// Renders `name` across terminal sizes, from empty to large, and
    /// across its whole timeline.
    fn renders_without_panicking(name: &str) {
        let style = find_style(name).expect("registered style");
        for (w, h) in [(80, 24), (200, 60), (10, 3), (1, 1), (0, 0)] {
            let mut run = style.start(7, w, h);
            let mut frame = Frame::new(w, h);
            for ms in [0, 16, 150, 900, 1600, 3000, 10_000, 600_000] {
                run.render(Duration::from_millis(ms), &mut frame);
            }
        }
    }

    #[test]
    fn classic_renders() {
        renders_without_panicking("classic");
    }

    #[test]
    fn decode_renders() {
        renders_without_panicking("decode");
    }

    #[test]
    fn plasma_renders() {
        renders_without_panicking("plasma");
    }

    #[test]
    fn dropin_renders() {
        renders_without_panicking("dropin");
    }
}
