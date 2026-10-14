//! The logo and footer text from `specs/about-easter-egg.md` (pure).

/// The six logo rows.
pub const LOGO: [&str; 6] = [
    " ██╗    ██╗ ██████╗ ██████╗ ██╗  ██╗",
    " ██║    ██║██╔═══██╗██╔══██╗██║ ██╔╝",
    " ██║ █╗ ██║██║   ██║██████╔╝█████╔╝",
    " ██║███╗██║██║   ██║██╔══██╗██╔═██╗",
    " ╚███╔███╔╝╚██████╔╝██║  ██║██║  ██╗",
    "  ╚══╝╚══╝  ╚═════╝ ╚═╝  ╚═╝╚═╝  ╚═╝",
];

/// The number of footer lines.
const FOOTER_LINES: usize = 3;

/// The logo's width in columns: its widest row, in chars (every logo glyph
/// is one column wide).
pub const WIDTH: usize = widest(&LOGO);

/// Rows in the whole block: the logo, a blank line, and the footer.
pub const BLOCK_HEIGHT: usize = LOGO.len() + 1 + FOOTER_LINES;

/// The chars in the widest of `rows` (counting UTF-8 lead bytes, so it can
/// run at compile time).
const fn widest(rows: &[&str]) -> usize {
    let mut max = 0;
    let mut r = 0;
    while r < rows.len() {
        let bytes = rows[r].as_bytes();
        let (mut chars, mut b) = (0, 0);
        while b < bytes.len() {
            if bytes[b] & 0xC0 != 0x80 {
                chars += 1;
            }
            b += 1;
        }
        if chars > max {
            max = chars;
        }
        r += 1;
    }
    max
}

/// The footer lines under the logo: `v<version>`, the attribution, and the
/// project URL.
#[must_use]
pub fn footer(version: &str) -> [String; FOOTER_LINES] {
    [
        format!("v{version}"),
        "Made with love by anoldguy".to_owned(),
        "github.com/anoldguy/work".to_owned(),
    ]
}

/// The columns before a footer `line` that centre it under the logo (0 when
/// it is wider).
#[must_use]
pub fn footer_pad(line: &str) -> usize {
    WIDTH.saturating_sub(line.chars().count()) / 2
}

#[cfg(test)]
mod tests {
    use super::{BLOCK_HEIGHT, LOGO, WIDTH, footer, footer_pad};

    #[test]
    fn width_is_the_widest_row_in_chars() {
        let widest = LOGO.iter().map(|l| l.chars().count()).max();
        assert_eq!(Some(WIDTH), widest);
        assert_eq!(WIDTH, 36);
        assert_eq!(BLOCK_HEIGHT, 10);
    }

    #[test]
    fn footer_lines_centre_under_the_logo() {
        assert_eq!(footer_pad("v0.1.0"), 15);
        assert_eq!(footer_pad("github.com/anoldguy/work"), 6);
        assert_eq!(footer_pad(&"x".repeat(50)), 0);
    }

    #[test]
    fn footer_matches_spec() {
        assert_eq!(
            footer("0.1.0"),
            [
                "v0.1.0",
                "Made with love by anoldguy",
                "github.com/anoldguy/work"
            ]
        );
    }

    #[test]
    fn logo_has_six_rows() {
        assert_eq!(LOGO.len(), 6);
        assert!(LOGO[0].starts_with(" ██╗    ██╗"));
    }
}
