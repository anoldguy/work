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

impl Rgb {
    /// A fully saturated colour at `hue` (wrapped into `[0, 1)`) around the
    /// colour wheel, each channel mapped as `channel * scale + offset` from
    /// `0.0..=1.0` and clamped to `0..=255`.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "bounded colour math: values are clamped to 0..=255 before narrowing"
    )]
    #[must_use]
    pub fn from_hue(hue: f64, scale: f64, offset: f64) -> Self {
        let sector = hue.rem_euclid(1.0) * 6.0;
        let mid = 1.0 - (sector % 2.0 - 1.0).abs();
        let (red, green, blue) = match sector as u8 {
            0 => (1.0, mid, 0.0),
            1 => (mid, 1.0, 0.0),
            2 => (0.0, 1.0, mid),
            3 => (0.0, mid, 1.0),
            4 => (mid, 0.0, 1.0),
            _ => (1.0, 0.0, mid),
        };
        let ch = |v: f64| (v * scale + offset).round().clamp(0.0, 255.0) as u8;
        Self {
            r: ch(red),
            g: ch(green),
            b: ch(blue),
        }
    }
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
    #[cfg(test)]
    #[must_use]
    pub(crate) fn to_text(&self) -> String {
        let mut out = String::with_capacity(self.cells.len() + usize::from(self.height));
        for row in self.cells.chunks(usize::from(self.width).max(1)) {
            out.extend(row.iter().map(|c| c.ch));
            out.push('\n');
        }
        out
    }

    /// One char per cell for colour snapshots: a cell's own glyph when it
    /// is text, otherwise a code for its background (or, failing that,
    /// foreground) colour, quantised to two bits per channel; a blank,
    /// uncoloured cell is a space.
    #[cfg(test)]
    pub(crate) fn to_color_map(&self) -> String {
        const CODES: &[u8; 64] =
            b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ+=";
        let code = |c: Rgb| {
            let i = usize::from(c.r >> 6) << 4 | usize::from(c.g >> 6) << 2 | usize::from(c.b >> 6);
            char::from(CODES[i])
        };
        let mut out = String::new();
        for row in self.cells.chunks(usize::from(self.width).max(1)) {
            out.extend(row.iter().map(|c| match (c.ch, c.bg.or(c.fg)) {
                (' ' | '▀', Some(colour)) => code(colour),
                (ch, _) => ch,
            }));
            out.push('\n');
        }
        out
    }
}

/// `n` as a `u16`, saturating at `u16::MAX`.
#[must_use]
pub(crate) fn to_u16(n: usize) -> u16 {
    u16::try_from(n).unwrap_or(u16::MAX)
}

/// The offset that centres `inner` cells in `outer` (0 when it does not
/// fit).
#[must_use]
pub(crate) fn center(outer: u16, inner: usize) -> u16 {
    outer.saturating_sub(to_u16(inner)) / 2
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::{Cell, Frame, Rgb, center, to_u16};

    #[test]
    fn from_hue_walks_the_wheel_and_wraps() {
        let rgb = |r, g, b| Rgb { r, g, b };
        assert_eq!(Rgb::from_hue(0.0, 255.0, 0.0), rgb(255, 0, 0));
        assert_eq!(Rgb::from_hue(1.0 / 3.0, 255.0, 0.0), rgb(0, 255, 0));
        assert_eq!(Rgb::from_hue(2.0 / 3.0, 255.0, 0.0), rgb(0, 0, 255));
        assert_eq!(Rgb::from_hue(1.0, 255.0, 0.0), rgb(255, 0, 0));
        assert_eq!(Rgb::from_hue(-0.5, 255.0, 0.0), rgb(0, 255, 255));
        assert_eq!(Rgb::from_hue(0.0, 200.0, 30.0), rgb(230, 30, 30));
    }

    #[test]
    fn center_splits_the_slack_and_clamps_at_zero() {
        assert_eq!(center(80, 36), 22);
        assert_eq!(center(37, 36), 0);
        assert_eq!(center(10, 36), 0);
        assert_eq!(center(10, usize::MAX), 0);
    }

    #[test]
    fn to_u16_saturates() {
        assert_eq!(to_u16(7), 7);
        assert_eq!(to_u16(70_000), u16::MAX);
    }

    #[test]
    fn color_map_shows_text_and_colour_codes() {
        let mut f = Frame::new(3, 1);
        f.put_str(0, 0, "a", Some(Rgb { r: 255, g: 0, b: 0 }), None);
        f.put_str(1, 0, " ", None, Some(Rgb { r: 0, g: 0, b: 255 }));
        assert_eq!(f.to_color_map(), "a3 \n");
    }

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
