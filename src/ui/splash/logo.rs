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
    let _ = version;
    unimplemented!("W1.9")
}
