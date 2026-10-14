//! [`Frame`]: a grid of styled cells that styles draw into (pure).

/// A 24-bit colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
}

/// One terminal cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    /// The character drawn (a single-width char).
    pub ch: char,
    /// Foreground colour; `None` is the terminal default.
    pub fg: Option<Rgb>,
    /// Background colour; `None` is the terminal default.
    pub bg: Option<Rgb>,
}

impl Cell {
    /// A space in default colours.
    pub const BLANK: Self = Self {
        ch: ' ',
        fg: None,
        bg: None,
    };
}

impl Default for Cell {
    fn default() -> Self {
        Self::BLANK
    }
}

/// A `width`×`height` grid of cells, row-major.
///
/// Invariant: `cells.len() == width * height`. Out-of-range writes are
/// ignored (clipped), so styles need not bounds-check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    width: u16,
    height: u16,
    cells: Vec<Cell>,
}

impl Frame {
    fn index(&self, x: u16, y: u16) -> Option<usize> {
        (x < self.width && y < self.height)
            .then(|| usize::from(y) * usize::from(self.width) + usize::from(x))
    }

    /// A blank frame.
    #[must_use]
    pub fn new(width: u16, height: u16) -> Self {
        let len = usize::from(width) * usize::from(height);
        Self {
            width,
            height,
            cells: vec![Cell::BLANK; len],
        }
    }

    /// Columns.
    #[must_use]
    pub const fn width(&self) -> u16 {
        self.width
    }

    /// Rows.
    #[must_use]
    pub const fn height(&self) -> u16 {
        self.height
    }

    /// Resets every cell to [`Cell::BLANK`].
    pub fn clear(&mut self) {
        self.cells.fill(Cell::BLANK);
    }

    /// The cell at column `x`, row `y`, if in range.
    #[must_use]
    pub fn get(&self, x: u16, y: u16) -> Option<&Cell> {
        self.index(x, y).and_then(|i| self.cells.get(i))
    }

    /// Sets the cell at column `x`, row `y`; ignored when out of range.
    pub fn set(&mut self, x: u16, y: u16, cell: Cell) {
        if let Some(slot) = self.index(x, y).and_then(|i| self.cells.get_mut(i)) {
            *slot = cell;
        }
    }

    /// Writes `text` from column `x` on row `y` with the given colours,
    /// clipped at the right edge.
    pub fn put_str(&mut self, x: u16, y: u16, text: &str, fg: Option<Rgb>, bg: Option<Rgb>) {
        for (dx, ch) in (0..=u16::MAX).zip(text.chars()) {
            let Some(cx) = x.checked_add(dx) else { break };
            if cx >= self.width {
                break;
            }
            self.set(cx, y, Cell { ch, fg, bg });
        }
    }

    /// The characters only, one line per row (for snapshots).
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = String::with_capacity(self.cells.len() + usize::from(self.height));
        for row in self.cells.chunks(usize::from(self.width).max(1)) {
            out.extend(row.iter().map(|c| c.ch));
            out.push('\n');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::{Cell, Frame, Rgb};

    #[test]
    fn new_frame_is_blank_and_sized() {
        let f = Frame::new(3, 2);
        assert_eq!((f.width(), f.height()), (3, 2));
        assert_eq!(f.get(2, 1), Some(&Cell::BLANK));
        assert_matches!(f.get(3, 0), None);
        assert_matches!(f.get(0, 2), None);
    }

    #[test]
    fn out_of_range_set_is_ignored() {
        let mut f = Frame::new(2, 2);
        let cell = Cell {
            ch: 'x',
            ..Cell::BLANK
        };
        f.set(2, 0, cell);
        f.set(0, 2, cell);
        assert_eq!(f, Frame::new(2, 2));
    }

    #[test]
    fn put_str_clips_at_right_edge() {
        let mut f = Frame::new(4, 1);
        let red = Rgb { r: 255, g: 0, b: 0 };
        f.put_str(2, 0, "abc", Some(red), None);
        assert_eq!(f.to_text(), "  ab\n");
        assert_eq!(f.get(3, 0).and_then(|c| c.fg), Some(red));
    }

    #[test]
    fn clear_resets_cells() {
        let mut f = Frame::new(2, 1);
        f.put_str(0, 0, "ab", None, None);
        f.clear();
        assert_eq!(f, Frame::new(2, 1));
    }

    #[test]
    fn to_text_has_one_line_per_row() {
        let mut f = Frame::new(2, 2);
        f.put_str(0, 1, "hi", None, None);
        assert_eq!(f.to_text(), "  \nhi\n");
    }
}
