//! The `decode` style: scrambled glyphs that lock in column by column, then a diagonal shimmer.

use crate::ui::splash::{BlankRun, Style, StyleRun};

/// The `decode` style.
#[derive(Debug)]
pub struct Decode;

impl Style for Decode {
    fn name(&self) -> &'static str {
        "decode"
    }

    fn start(&self, seed: u64, w: u16, h: u16) -> Box<dyn StyleRun> {
        // W2.9 replaces this blank placeholder with the real animation.
        let _ = (seed, w, h);
        Box::new(BlankRun)
    }
}
