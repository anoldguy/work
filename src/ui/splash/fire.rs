//! The `fire` style: a demoscene heat-map fire below the logo (stateful).

use crate::ui::splash::{BlankRun, Style, StyleRun};

/// The `fire` style.
#[derive(Debug)]
pub struct Fire;

impl Style for Fire {
    fn name(&self) -> &'static str {
        "fire"
    }

    fn start(&self, seed: u64, w: u16, h: u16) -> Box<dyn StyleRun> {
        // W2.10 replaces this blank placeholder with the real animation.
        let _ = (seed, w, h);
        Box::new(BlankRun)
    }
}
