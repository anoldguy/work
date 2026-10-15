//! `work`: navigate and sync your GitHub repositories.
//!
//! The crate is a library with a thin binary. [`run`] is the entry point:
//! it captures the environment, creates the single [`ui::Ui`], parses the
//! command line, and dispatches in two stages:
//!
//! 1. Commands that never build an [`app::App`] (so a broken config cannot
//!    block them): `version`, `activate`, `sh-editor`, `about`, plus (with
//!    `config::Locations`) `completions`, `config path`, `config --edit`, `auth login|logout` and
//!    `list --complete`.
//! 2. Everything else, with an [`app::App`].
//!
//! `Locations` is computed at most once, only for commands that need it, so
//! `work version` works without `HOME`.
//!
//! Layering: `core` and `domain` are pure; adapters (`git`, `gh`, `github`,
//! `proc`, `platform`, stores) do the I/O; services report progress through
//! `progress`, never `ui`; only `ui` writes to the terminal.

pub mod app;
pub mod auth;
pub mod cache;
pub mod cli;
pub mod commands;
pub mod config;
pub mod core;
pub mod domain;
pub mod env;
pub mod error;
pub mod frecency;
pub(crate) mod fsutil;
pub mod gh;
pub mod git;
pub mod github;
pub mod platform;
pub mod pool;
pub mod proc;
pub mod progress;
pub mod shell;
pub mod sync;
#[cfg(test)]
pub mod testing;
pub mod text;
pub mod ui;

pub use error::Error;

use std::ffi::OsString;
use std::process::ExitCode;

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser};

use cli::{Cli, Command, ConfigAction};
use commands::nav::Invocation;
use commands::{
    about, auth as auth_cmd, completions, config as config_cmd, favorite, list, meta, nav, setup,
    sync as sync_cmd, update,
};
use config::Locations;

/// Runs `work` with `args` (including the program name).
///
/// `--help` and `--version` are printed here and return success. The caller
/// (`main`) prints an `Err` as `work: <summary>` plus `  - <item>` lines
/// from [`Error::list_items`] (unless [`Error::is_silent`]) and exits with
/// [`Error::exit_code`].
///
/// # Errors
///
/// Whatever the command fails with; usage errors become [`Error::Usage`].
pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<ExitCode, Error> {
    let env = env::EnvSnapshot::capture();
    let mut ui = ui::Ui::new(&env);
    let args: Vec<OsString> = args.into_iter().collect();
    let command = match Cli::try_parse_from(&args) {
        Ok(cli) => cli.command,
        Err(err)
            if matches!(
                err.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            print_block(&mut ui, &err.to_string())?;
            return Ok(ExitCode::SUCCESS);
        }
        Err(err) => return Err(Error::from_clap_in(&err, &help_path(&args))),
    };
    let Some(command) = command else {
        print_block(&mut ui, &Cli::command().render_help().to_string())?;
        return Ok(ExitCode::SUCCESS);
    };

    // Stage 1: never builds an `App`, so a broken config cannot block it.
    let (command, locations) = match stage1(command, &env, &mut ui)? {
        Stage::Done(code) => {
            ui.flush()?;
            return Ok(code);
        }
        Stage::Silent(code) => {
            // Completion candidates: a failed flush must not print an error.
            let _ = ui.flush();
            return Ok(code);
        }
        Stage::Next(command, locations) => (command, locations),
    };

    // Stage 2: everything else runs with an `App`.
    let mut app = app::App::new(env, locations, ui)?;
    let code = stage2(&mut app, command)?;
    app.ui.flush()?;
    Ok(code)
}

/// What stage 1 decided.
enum Stage {
    /// The command ran.
    Done(ExitCode),
    /// The command ran and every output error is swallowed.
    Silent(ExitCode),
    /// The command needs an `App`; `Locations` is computed once and handed on.
    Next(AppCommand, Locations),
}

/// The commands that need an `App`: [`Command`] minus everything stage 1
/// handles, with `config` split by what it does. Having no stage-1 variants
/// makes the stage-2 `match` exhaustive without impossible arms.
enum AppCommand {
    Cd(cli::CdArgs, Invocation),
    On(cli::ResolveArgs, Invocation),
    Edit(cli::ResolveArgs),
    Clone(cli::ResolveArgs),
    Browse(cli::ResolveArgs),
    Open(cli::ResolveArgs),
    Pull(cli::ResolveArgs),
    Sync(cli::SyncArgs),
    List(cli::ListArgs),
    Update(cli::UpdateArgs),
    Favorite(cli::FavoriteArgs),
    Setup(cli::SetupArgs),
    ConfigShow,
    ConfigDefault(Option<domain::Domain>),
    Auth(cli::AuthArgs),
}

fn stage1(command: Command, env: &env::EnvSnapshot, ui: &mut ui::Ui) -> Result<Stage, Error> {
    let next = match command {
        Command::Version => return meta::version(ui).map(Stage::Done),
        Command::ShEditor => return nav::sh_editor(env, ui).map(Stage::Done),
        Command::Activate(args) => return meta::activate(env, ui, &args).map(Stage::Done),
        Command::About(args) => return about::run(ui, &args).map(Stage::Done),
        Command::Completions(args) => {
            let locations = Locations::from_env(env)?;
            return completions::run(env, &locations, ui, &args).map(Stage::Done);
        }
        Command::Config(args) if args.edit => {
            return config_cmd::edit(env, &Locations::from_env(env)?).map(Stage::Done);
        }
        Command::Config(args) => match args.action {
            Some(ConfigAction::Path) => {
                return config_cmd::path(&Locations::from_env(env)?, ui).map(Stage::Done);
            }
            Some(ConfigAction::Default { domain }) => AppCommand::ConfigDefault(domain),
            None => AppCommand::ConfigShow,
        },
        Command::List(args) if args.complete => {
            // Best effort: stay silent on failure.
            let code = Locations::from_env(env).map_or(ExitCode::SUCCESS, |locations| {
                list::complete(&locations, ui)
            });
            return Ok(Stage::Silent(code));
        }
        Command::List(args) => AppCommand::List(args),
        Command::Cd(args) => AppCommand::Cd(args, Invocation::Direct),
        Command::ShCd(args) => AppCommand::Cd(args, Invocation::Wrapped),
        Command::On(args) => AppCommand::On(args, Invocation::Direct),
        Command::ShOn(args) => AppCommand::On(args, Invocation::Wrapped),
        Command::Edit(args) => AppCommand::Edit(args),
        Command::Clone(args) => AppCommand::Clone(args),
        Command::Browse(args) => AppCommand::Browse(args),
        Command::Open(args) => AppCommand::Open(args),
        Command::Pull(args) => AppCommand::Pull(args),
        Command::Sync(args) => AppCommand::Sync(args),
        Command::Update(args) => AppCommand::Update(args),
        Command::Favorite(args) => AppCommand::Favorite(args),
        Command::Setup(args) => AppCommand::Setup(args),
        Command::Auth(args) => match args.action {
            cli::AuthAction::Status => AppCommand::Auth(args),
            ref action => {
                auth_cmd::login_logout(&gh::ProcessGh, ui, action)?;
                return Ok(Stage::Done(ExitCode::SUCCESS));
            }
        },
    };
    Ok(Stage::Next(next, Locations::from_env(env)?))
}

fn stage2(app: &mut app::App, command: AppCommand) -> Result<ExitCode, Error> {
    match command {
        AppCommand::Cd(args, via) => nav::cd(app, &args, via),
        AppCommand::On(args, via) => nav::on(app, &args, via),
        AppCommand::Edit(args) => nav::edit(app, &args),
        AppCommand::Clone(args) => nav::clone(app, &args),
        AppCommand::Browse(args) => nav::browse(app, &args),
        AppCommand::Open(args) => nav::open(app, &args),
        AppCommand::Pull(args) => sync_cmd::pull(app, &args),
        AppCommand::Sync(args) => sync_cmd::sync(app, &args),
        AppCommand::List(args) => list::run(app, &args),
        AppCommand::Update(args) => update::run(app, &args),
        AppCommand::Favorite(args) => favorite::run(app, &args),
        AppCommand::Setup(args) => setup::run(app, &args),
        AppCommand::ConfigShow => config_cmd::show(app),
        AppCommand::ConfigDefault(domain) => config_cmd::set_default(app, domain),
        AppCommand::Auth(args) => auth_cmd::run(app, &args),
    }
}

/// Prints already-rendered multi-line text (help, version) to stdout.
fn print_block(ui: &mut ui::Ui, text: &str) -> Result<(), Error> {
    ui.stdout_line(text.trim_end_matches('\n'))?;
    ui.flush()?;
    Ok(())
}

/// `work` followed by the subcommands `args` reached, for the
/// `(see '<path> --help')` hint. Stops at the first word that is not a
/// subcommand, so untrusted words never appear in the hint. Hidden
/// `sh-<cmd>` wrapper subcommands are shown as `<cmd>`.
pub(crate) fn help_path<S: AsRef<std::ffi::OsStr>>(args: &[S]) -> String {
    let mut path = String::from("work");
    let mut command = Cli::command();
    for word in args.iter().skip(1) {
        let Some(sub) = word
            .as_ref()
            .to_str()
            .and_then(|name| command.find_subcommand(name))
        else {
            if word.as_ref().to_str().is_some_and(|w| w.starts_with('-')) {
                continue;
            }
            break;
        };
        // The wrapper runs hidden `sh-<cmd>` for the user's `<cmd>`: name
        // the command they typed.
        let name = sub.get_name();
        let shown = name
            .strip_prefix("sh-")
            .filter(|_| sub.is_hide_set())
            .unwrap_or(name);
        path.push(' ');
        path.push_str(shown);
        command = sub.clone();
    }
    path
}
