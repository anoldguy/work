//! Diff renderer: writes only the cells that changed since the last frame.

use std::io::{self, Write};

use crate::ui::splash::{Cell, Frame, Rgb};

/// Remembers the last frame drawn so the next draw can be a diff.
#[derive(Debug, Default)]
pub struct Renderer {
    prev: Option<Frame>,
}

/// The colours last written to the terminal.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Look {
    fg: Option<Rgb>,
    bg: Option<Rgb>,
}

const DEFAULT_LOOK: Look = Look { fg: None, bg: None };

impl Renderer {
    /// A renderer that will draw its first frame in full.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Draws `frame` to `out` (cursor moves plus styled text), colourless
    /// when `color` is false, and remembers it.
    ///
    /// The first frame (and the first after a size change) clears the screen
    /// and writes every non-blank cell; later frames write only changed
    /// cells, moving the cursor only where a run of changes is not
    /// contiguous.
    ///
    /// # Errors
    ///
    /// Any write error.
    pub fn draw(&mut self, frame: &Frame, color: bool, out: &mut dyn Write) -> io::Result<()> {
        let mut buf = Vec::new();
        let prev = self
            .prev
            .as_ref()
            .filter(|p| p.width() == frame.width() && p.height() == frame.height());
        if prev.is_none() {
            buf.extend_from_slice(b"\x1b[0m\x1b[2J");
        }
        let mut cursor: Option<(u16, u16)> = None;
        let mut look = DEFAULT_LOOK;
        for y in 0..frame.height() {
            for x in 0..frame.width() {
                let Some(&cell) = frame.get(x, y) else {
                    continue;
                };
                let old = prev.and_then(|p| p.get(x, y)).copied();
                if !cell_changed(old, cell, color) {
                    continue;
                }
                if cursor != Some((x, y)) {
                    write!(buf, "\x1b[{};{}H", y.saturating_add(1), x.saturating_add(1))?;
                }
                if color {
                    let want = Look {
                        fg: cell.fg,
                        bg: cell.bg,
                    };
                    write_look(&mut buf, &mut look, want)?;
                }
                write!(buf, "{}", cell.ch)?;
                cursor = x.checked_add(1).map(|nx| (nx, y));
            }
        }
        if look != DEFAULT_LOOK {
            buf.extend_from_slice(b"\x1b[0m");
        }
        out.write_all(&buf)?;
        out.flush()?;
        self.prev = Some(frame.clone());
        Ok(())
    }
}

/// Whether `cell` must be written given what the terminal shows (`old`;
/// `None` means freshly cleared, i.e. blank).
fn cell_changed(old: Option<Cell>, cell: Cell, color: bool) -> bool {
    let old = old.unwrap_or(Cell::BLANK);
    if color {
        old != cell
    } else {
        old.ch != cell.ch
    }
}

/// Emits the SGR sequences moving the terminal from `current` to `want`,
/// touching only the colour that changed (full-field styles change the
/// background of nearly every cell and nothing else).
fn write_look(buf: &mut Vec<u8>, current: &mut Look, want: Look) -> io::Result<()> {
    if current.fg != want.fg {
        match want.fg {
            Some(c) => write!(buf, "\x1b[38;2;{};{};{}m", c.r, c.g, c.b)?,
            None => buf.extend_from_slice(b"\x1b[39m"),
        }
    }
    if current.bg != want.bg {
        match want.bg {
            Some(c) => write!(buf, "\x1b[48;2;{};{};{}m", c.r, c.g, c.b)?,
            None => buf.extend_from_slice(b"\x1b[49m"),
        }
    }
    *current = want;
    Ok(())
}
