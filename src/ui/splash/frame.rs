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
    /// A blank frame.
    #[must_use]
    pub fn new(width: u16, height: u16) -> Self {
        let _ = (width, height);
        unimplemented!("W1.9")
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
        let _ = &mut self.cells;
        unimplemented!("W1.9")
    }

    /// The cell at column `x`, row `y`, if in range.
    #[must_use]
    pub fn get(&self, x: u16, y: u16) -> Option<&Cell> {
        let _ = (self, x, y);
        unimplemented!("W1.9")
    }

    /// Sets the cell at column `x`, row `y`; ignored when out of range.
    pub fn set(&mut self, x: u16, y: u16, cell: Cell) {
        let _ = (self, x, y, cell);
        unimplemented!("W1.9")
    }

    /// Writes `text` from column `x` on row `y` with the given colours,
    /// clipped at the right edge.
    pub fn put_str(&mut self, x: u16, y: u16, text: &str, fg: Option<Rgb>, bg: Option<Rgb>) {
        let _ = (self, x, y, text, fg, bg);
        unimplemented!("W1.9")
    }

    /// The characters only, one line per row (for snapshots).
    #[must_use]
    pub fn to_text(&self) -> String {
        let _ = self;
        unimplemented!("W1.9")
    }
}
