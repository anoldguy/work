//! The `about` splash: a registry of animation styles that render into a
//! pure [`Frame`] buffer.
//!
//! Styles are time-based (a dropped frame skips ahead) and draw randomness
//! only from the seed they are started with, so tests render deterministic
//! frames at a fixed seed, time and size. `Ui::splash` (in `splash/run.rs`)
//! plays one on the terminal.

pub mod classic;
pub mod decode;
pub mod dropin;
pub mod fire;
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

/// One running animation. Stateful runs (such as `fire`) keep their state
/// here.
pub trait StyleRun {
    /// Draws the frame at time `t` since the start into `frame` (which has
    /// the run's size).
    fn render(&mut self, t: Duration, frame: &mut Frame);
}

/// A run that draws nothing: the placeholder for a style whose animation has
/// not been written yet. Leaves the frame blank.
#[derive(Debug, Clone, Copy, Default)]
pub struct BlankRun;

impl StyleRun for BlankRun {
    fn render(&mut self, t: Duration, frame: &mut Frame) {
        let _ = (t, frame);
    }
}

/// Every style, in registry order.
pub static STYLES: &[&dyn Style] = &[
    &classic::Classic,
    &decode::Decode,
    &plasma::Plasma,
    &fire::Fire,
    &dropin::Dropin,
];

/// The style called `name`, if any.
#[must_use]
pub fn find_style(name: &str) -> Option<&'static dyn Style> {
    STYLES.iter().copied().find(|s| s.name() == name)
}

/// Every style name, in registry order (for the unknown-style list error).
#[must_use]
pub fn style_names() -> Vec<&'static str> {
    STYLES.iter().map(|s| s.name()).collect()
}

#[cfg(test)]
mod tests {
    use super::{find_style, style_names};

    #[test]
    fn registry_lists_the_five_styles_in_order() {
        assert_eq!(
            style_names(),
            ["classic", "decode", "plasma", "fire", "dropin"]
        );
        assert!(find_style("fire").is_some());
        assert!(find_style("Fire").is_none());
    }
}
