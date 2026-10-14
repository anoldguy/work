//! Diff renderer: writes only the cells that changed since the last frame.

use std::io::{self, Write};

use crate::ui::splash::Frame;

/// Remembers the last frame drawn so the next draw can be a diff.
#[derive(Debug, Default)]
pub struct Renderer {
    prev: Option<Frame>,
}

impl Renderer {
    /// A renderer that will draw its first frame in full.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Draws `frame` to `out` (cursor moves plus styled text), colourless
    /// when `color` is false, and remembers it.
    ///
    /// # Errors
    ///
    /// Any write error.
    pub fn draw(&mut self, frame: &Frame, color: bool, out: &mut dyn Write) -> io::Result<()> {
        let _ = (&mut self.prev, frame, color, out);
        unimplemented!("W2.13")
    }
}
