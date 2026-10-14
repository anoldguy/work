//! The `dropin` style: logo columns that fall in on a spring, then the footer springs up.

use crate::ui::splash::{BlankRun, Style, StyleRun};

/// The `dropin` style.
#[derive(Debug)]
pub struct Dropin;

impl Style for Dropin {
    fn name(&self) -> &'static str {
        "dropin"
    }

    fn start(&self, seed: u64, w: u16, h: u16) -> Box<dyn StyleRun> {
        // W2.9 replaces this blank placeholder with the real animation.
        let _ = (seed, w, h);
        Box::new(BlankRun)
    }
}
