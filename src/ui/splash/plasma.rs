//! The `plasma` style: a full-screen sine-field plasma behind the logo.

use crate::ui::splash::{BlankRun, Style, StyleRun};

/// The `plasma` style.
#[derive(Debug)]
pub struct Plasma;

impl Style for Plasma {
    fn name(&self) -> &'static str {
        "plasma"
    }

    fn start(&self, seed: u64, w: u16, h: u16) -> Box<dyn StyleRun> {
        // W2.10 replaces this blank placeholder with the real animation.
        let _ = (seed, w, h);
        Box::new(BlankRun)
    }
}
