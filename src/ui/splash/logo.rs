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

/// The footer lines under the logo: `v<version>`, the attribution, and the
/// project URL.
#[must_use]
pub fn footer(version: &str) -> [String; 3] {
    [
        format!("v{version}"),
        "Made with love by anoldguy".to_owned(),
        "github.com/anoldguy/work".to_owned(),
    ]
}

#[cfg(test)]
mod tests {
    use super::{LOGO, footer};

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
