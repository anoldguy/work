//! `completions [shell] [--install]` (stage 1: no config needed).

use std::fs;
use std::process::ExitCode;

use clap::CommandFactory;

use crate::cli::{Cli, CompletionsArgs};
use crate::config::Locations;
use crate::env::EnvSnapshot;
use crate::error::Error;
use crate::shell::completions::homebrew_prefix;
use crate::shell::{Shell, ShellError, completion_script, detect, install_path};
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
    args: &CompletionsArgs,
) -> Result<ExitCode, Error> {
    let &CompletionsArgs { shell, install } = args;
    let shell = shell.unwrap_or_else(|| detect(env));
    let script = completion_script(shell, &mut Cli::command());
    if !install {
        ui.stdout_line(script.trim_end_matches('\n'))?;
        return Ok(ExitCode::SUCCESS);
    }
    let brew = homebrew_prefix(env);
    let path = install_path(shell, locations, brew.as_deref());
    let install_err = |source| ShellError::Install {
        path: path.clone(),
        source,
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(install_err)?;
    }
    fs::write(&path, script).map_err(install_err)?;
    ui.stderr_line(&format!(
        "installed {shell} completions to {}",
        path.display()
    ))?;
    if shell == Shell::Zsh
        && brew.is_none()
        && let Some(dir) = path.parent()
    {
        ui.stderr_line(&format!(
            "add the directory to your fpath before compinit: fpath=({} $fpath)",
            dir.display()
        ))?;
    }
    Ok(ExitCode::SUCCESS)
}
