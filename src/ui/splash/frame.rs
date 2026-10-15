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
