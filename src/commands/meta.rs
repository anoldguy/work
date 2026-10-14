//! `version`, `commands --sh`, and `activate` (stage 1: no config needed).

use std::process::ExitCode;

use crate::cli::ActivateArgs;
use crate::env::EnvSnapshot;
use crate::error::Error;
use crate::shell::{activation_script, detect};
use crate::ui::Ui;

/// The commands the wrapper intercepts, printed by `commands --sh`.
pub const SH_COMMANDS: [&str; 2] = ["cd", "on"];

/// Prints `CARGO_PKG_VERSION`.
///
/// # Errors
///
/// A broken pipe.
pub fn version(ui: &mut Ui) -> Result<ExitCode, Error> {
    ui.stdout_line(env!("CARGO_PKG_VERSION"))?;
    Ok(ExitCode::SUCCESS)
}

/// `commands --sh`: prints [`SH_COMMANDS`], one per line, and nothing else.
/// The parser rejects `commands` without `--sh`, so there is nothing to
/// check here.
///
/// # Errors
///
/// A broken pipe.
pub fn commands(ui: &mut Ui) -> Result<ExitCode, Error> {
    for name in SH_COMMANDS {
        ui.stdout_line(name)?;
    }
    Ok(ExitCode::SUCCESS)
}

/// `activate [shell]`: prints the embedded wrapper for the given or detected
/// shell.
///
/// # Errors
///
/// A broken pipe. An unknown shell name is rejected by the argument parser
/// before this runs.
pub fn activate(
    env: &EnvSnapshot,
    ui: &mut Ui,
    ActivateArgs { shell }: &ActivateArgs,
) -> Result<ExitCode, Error> {
    let shell = shell.unwrap_or_else(|| detect(env));
    // The embedded scripts end with exactly one newline, which `stdout_line`
    // supplies, so the output is the script byte-for-byte.
    ui.stdout_line(activation_script(shell).trim_end_matches('\n'))?;
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::SH_COMMANDS;
    use crate::cli::Cli;

    #[test]
    fn every_sh_command_has_a_hidden_sh_subcommand() {
        let cli = Cli::command();
        for name in SH_COMMANDS {
            let sub = cli
                .find_subcommand(format!("sh-{name}"))
                .unwrap_or_else(|| panic!("missing subcommand sh-{name}"));
            assert!(sub.is_hide_set(), "sh-{name} must be hidden");
        }
    }
}
