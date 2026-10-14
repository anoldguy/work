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

#[cfg(test)]
mod tests {
    use super::Renderer;
    use crate::ui::splash::{Frame, Rgb};

    const RED: Rgb = Rgb { r: 255, g: 0, b: 0 };
    const BLUE: Rgb = Rgb { r: 0, g: 0, b: 255 };

    fn draw(r: &mut Renderer, f: &Frame, color: bool) -> String {
        let mut out = Vec::new();
        r.draw(f, color, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn first_frame_clears_and_writes_non_blank_cells() {
        let mut f = Frame::new(5, 2);
        f.put_str(1, 1, "ab", None, None);
        let s = draw(&mut Renderer::new(), &f, false);
        assert_eq!(s, "\x1b[0m\x1b[2J\x1b[2;2Hab");
    }

    #[test]
    fn identical_frame_writes_nothing() {
        let mut f = Frame::new(5, 2);
        f.put_str(0, 0, "hi", Some(RED), None);
        let mut r = Renderer::new();
        draw(&mut r, &f, true);
        assert_eq!(draw(&mut r, &f, true), "");
    }

    #[test]
    fn only_changed_cells_are_rewritten() {
        let mut f = Frame::new(6, 1);
        f.put_str(0, 0, "abcdef", None, None);
        let mut r = Renderer::new();
        draw(&mut r, &f, false);
        f.put_str(1, 0, "X", None, None);
        f.put_str(4, 0, "YZ", None, None);
        assert_eq!(draw(&mut r, &f, false), "\x1b[1;2HX\x1b[1;5HYZ");
    }

    #[test]
    fn contiguous_changes_share_one_cursor_move() {
        let mut f = Frame::new(4, 2);
        let mut r = Renderer::new();
        draw(&mut r, &f, false);
        f.put_str(2, 0, "ab", None, None);
        f.put_str(0, 1, "cd", None, None);
        assert_eq!(draw(&mut r, &f, false), "\x1b[1;3Hab\x1b[2;1Hcd");
    }

    #[test]
    fn colour_changes_are_ignored_without_color() {
        let mut f = Frame::new(3, 1);
        f.put_str(0, 0, "abc", None, None);
        let mut r = Renderer::new();
        draw(&mut r, &f, false);
        f.put_str(0, 0, "abc", Some(RED), None);
        assert_eq!(draw(&mut r, &f, false), "");
    }

    #[test]
    fn colour_is_emitted_when_enabled_and_reset_at_the_end() {
        let mut f = Frame::new(3, 1);
        f.put_str(0, 0, "ab", Some(RED), None);
        let s = draw(&mut Renderer::new(), &f, true);
        assert_eq!(s, "\x1b[0m\x1b[2J\x1b[1;1H\x1b[38;2;255;0;0mab\x1b[0m");
    }

    #[test]
    fn colour_only_change_redraws_the_cell() {
        let mut f = Frame::new(3, 1);
        f.put_str(0, 0, "abc", None, None);
        let mut r = Renderer::new();
        draw(&mut r, &f, true);
        f.put_str(1, 0, "b", Some(RED), None);
        assert_eq!(
            draw(&mut r, &f, true),
            "\x1b[1;2H\x1b[38;2;255;0;0mb\x1b[0m"
        );
    }

    #[test]
    fn only_the_colour_that_changes_is_written() {
        let mut f = Frame::new(3, 1);
        f.put_str(0, 0, "a", Some(RED), Some(BLUE));
        f.put_str(1, 0, "b", Some(RED), Some(RED));
        f.put_str(2, 0, "c", None, Some(RED));
        assert_eq!(
            draw(&mut Renderer::new(), &f, true),
            concat!(
                "\x1b[0m\x1b[2J\x1b[1;1H",
                "\x1b[38;2;255;0;0m\x1b[48;2;0;0;255ma",
                "\x1b[48;2;255;0;0mb",
                "\x1b[39mc",
                "\x1b[0m"
            )
        );
    }

    #[test]
    fn size_change_redraws_in_full() {
        let mut r = Renderer::new();
        draw(&mut r, &Frame::new(3, 1), false);
        let mut f = Frame::new(4, 1);
        f.put_str(0, 0, "z", None, None);
        assert_eq!(draw(&mut r, &f, false), "\x1b[0m\x1b[2J\x1b[1;1Hz");
    }
}
