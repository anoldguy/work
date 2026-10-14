//! The `classic` style: the logo with a rainbow gradient that scrolls one row per 150 ms.

use crate::ui::splash::{Style, StyleRun};

/// The `classic` style.
#[derive(Debug)]
pub struct Classic;

impl Style for Classic {
    fn name(&self) -> &'static str {
        "classic"
    }

    fn start(&self, seed: u64, w: u16, h: u16) -> Box<dyn StyleRun> {
        let _ = (seed, w, h);
        unimplemented!("W1.9")
    }
}
