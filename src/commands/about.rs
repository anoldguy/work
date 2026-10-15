//! `about [--style name]` (no config needed).

use std::num::NonZeroU32;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::cli::AboutArgs;
use crate::error::Error;
use crate::shell::ShellError;
use crate::ui::splash::logo::{LOGO, footer, footer_pad};
use crate::ui::splash::rng::Rng;
use crate::ui::splash::{STYLES, Style, find_style, style_names};
use crate::ui::{Mode, Ui};

/// Mixed into the seed before choosing a style, so the choice is not
/// correlated with the chosen style's own first draws from the same seed.
const STYLE_SALT: u64 = 0xA076_1D64_78BD_642F;

/// Runs the splash at 60 fps in the alternate screen until a key is pressed,
/// using `--style` or a random style seeded from time XOR pid. Without a TTY,
/// writes the static logo and footer to stderr.
///
/// # Errors
///
/// `ShellError::UnknownStyle` (a list error), or terminal failures.
pub fn run(ui: &mut Ui, args: &AboutArgs) -> Result<ExitCode, Error> {
    let AboutArgs { style } = args;
    let seed = entropy_seed();
    let style = pick_style(style.as_deref(), seed)?;
    if ui.mode() == Mode::Plain {
        for line in static_lines() {
            ui.stderr_line(&line)?;
        }
        return Ok(ExitCode::SUCCESS);
    }
    ui.splash(style, seed)?;
    Ok(ExitCode::SUCCESS)
}

/// Time XOR pid: enough entropy for choosing a splash.
fn entropy_seed() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
    nanos ^ u64::from(std::process::id())
}

/// The named style, or a random one from `seed` when `name` is `None`.
fn pick_style(name: Option<&str>, seed: u64) -> Result<&'static dyn Style, ShellError> {
    if let Some(given) = name {
        return find_style(given).ok_or_else(|| ShellError::UnknownStyle {
            given: given.to_owned(),
            valid: style_names(),
        });
    }
    let count = u32::try_from(STYLES.len()).unwrap_or(u32::MAX);
    let bound = NonZeroU32::new(count).unwrap_or(NonZeroU32::MIN);
    let index = Rng::new(seed ^ STYLE_SALT).below(bound) as usize;
    Ok(STYLES
        .get(index)
        .copied()
        .unwrap_or(&crate::ui::splash::classic::Classic))
}

/// The logo, a blank line and the footer (centred under the logo), for
/// stderr without a terminal.
fn static_lines() -> Vec<String> {
    let mut lines: Vec<String> = LOGO.iter().map(|l| (*l).to_owned()).collect();
    lines.push(String::new());
    for line in footer(env!("CARGO_PKG_VERSION")) {
        lines.push(format!("{}{line}", " ".repeat(footer_pad(&line))));
    }
    lines
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::{pick_style, static_lines};
    use crate::shell::ShellError;
    use crate::ui::splash::STYLES;

    #[test]
    fn unknown_style_lists_the_valid_names() {
        let err = pick_style(Some("nope"), 0).err().unwrap();
        assert_matches!(&err, ShellError::UnknownStyle { given, valid }
            if given == "nope" && valid.len() == STYLES.len());
    }

    #[test]
    fn random_style_is_deterministic_per_seed_and_covers_the_registry() {
        let name = |seed| pick_style(None, seed).unwrap().name();
        assert_eq!(name(42), name(42));
        let seen: std::collections::HashSet<_> = (0..200).map(name).collect();
        assert_eq!(seen.len(), STYLES.len());
    }

    #[test]
    fn static_logo_ends_with_the_footer() {
        let lines = static_lines();
        assert_eq!(lines.len(), 10);
        assert!(lines[0].starts_with(" ██╗"));
        assert_eq!(lines[9].trim(), "github.com/anoldguy/work");
        // `v<version>` centred under the 36-column logo.
        let version = format!("v{}", env!("CARGO_PKG_VERSION"));
        let pad = (36 - version.len()) / 2;
        assert_eq!(lines[7], format!("{}{version}", " ".repeat(pad)));
    }
}
