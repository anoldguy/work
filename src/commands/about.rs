//! `about [--style name]` (stage 1: no config needed).

use std::process::ExitCode;

use crate::cli::AboutArgs;
use crate::env::EnvSnapshot;
use crate::error::Error;
use crate::ui::Ui;

/// Runs the splash at 60 fps in the alternate screen until a key is pressed,
/// using `--style` or a random style seeded from time XOR pid. Without a TTY,
/// writes the static logo and footer to stderr.
///
/// # Errors
///
/// `ShellError::UnknownStyle` (a list error), or terminal failures.
pub fn run(env: &EnvSnapshot, ui: &mut Ui, args: AboutArgs) -> Result<ExitCode, Error> {
    let _ = (env, ui, args);
    unimplemented!("W2.13")
}
