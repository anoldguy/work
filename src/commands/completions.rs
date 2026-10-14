//! `completions [shell] [--install]` (stage 1: no config needed).

use std::process::ExitCode;

use crate::cli::CompletionsArgs;
use crate::config::Locations;
use crate::env::EnvSnapshot;
use crate::error::Error;
use crate::ui::Ui;

/// Prints the completion script, or with `--install` writes it to the
/// shell's completion directory (and prints a zsh `fpath` hint on stderr
/// outside Homebrew).
///
/// # Errors
///
/// `ShellError::Install`; a broken pipe.
pub fn run(
    env: &EnvSnapshot,
    locations: &Locations,
    ui: &mut Ui,
    args: CompletionsArgs,
) -> Result<ExitCode, Error> {
    let _ = (env, locations, ui, args);
    unimplemented!("W2.11")
}
