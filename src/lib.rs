//! `work`: navigate and sync your GitHub repositories.
//!
//! The crate is a library with a thin binary. [`run`] is the entry point:
//! it captures the environment, creates the single [`ui::Ui`], parses the
//! command line, and dispatches with one `match`. Only the commands that
//! need a config build an [`app::App`]; `Locations` is computed at most
//! once, only for commands that need it, so `work version` works without
//! `HOME`.
//!
//! Layering: `core` and `domain` are pure; adapters (`git`, `gh`,
//! `proc`, `platform`, stores) do the I/O; services report progress through
//! `progress`, never `ui`; only `ui` writes to the terminal.

pub mod app;
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
pub mod platform;
pub mod pool;
pub mod proc;
pub mod progress;
pub mod shell;
pub mod sync;
pub mod text;
pub mod ui;

pub use error::Error;

use std::ffi::OsString;
use std::process::ExitCode;

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser};

use app::App;
use cli::{AuthAction, Cli, Command, ConfigAction};
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

    dispatch(command, env, &mut ui)
}

/// Runs `command` and flushes stdout. Only the arms that need a config
/// build an [`App`], so a broken config cannot block the others.
fn dispatch(command: Command, env: env::EnvSnapshot, ui: &mut ui::Ui) -> Result<ExitCode, Error> {
    let code = match command {
        Command::Version => meta::version(ui),
        Command::ShEditor => nav::sh_editor(&env, ui),
        Command::Activate(args) => meta::activate(&env, ui, &args),
        Command::About(args) => about::run(ui, &args),
        Command::Completions(args) => {
            completions::run(&env, &Locations::from_env(&env)?, ui, &args)
        }
        Command::Config(args) if args.edit => {
            config_cmd::edit(&env, &Locations::from_env(&env)?, ui)
        }
        Command::Config(args) => match args.action {
            Some(ConfigAction::Path) => config_cmd::path(&Locations::from_env(&env)?, ui),
            Some(ConfigAction::Default { domain }) => {
                config_cmd::set_default(&App::load(env)?, ui, domain)
            }
            None => config_cmd::show(&App::load(env)?, ui),
        },
        Command::List(args) if args.complete => {
            // Completion candidates: no failure, including a failed flush,
            // may print an error.
            let code = Locations::from_env(&env).map_or(ExitCode::SUCCESS, |locations| {
                list::complete(&locations, ui)
            });
            let _ = ui.flush();
            return Ok(code);
        }
        Command::List(args) => list::run(&App::load(env)?, ui, &args),
        Command::Cd(args) => nav::cd(&App::load(env)?, ui, &args, Invocation::Direct),
        Command::ShCd(args) => nav::cd(&App::load(env)?, ui, &args, Invocation::Wrapped),
        Command::On(args) => nav::on(&App::load(env)?, ui, &args, Invocation::Direct),
        Command::ShOn(args) => nav::on(&App::load(env)?, ui, &args, Invocation::Wrapped),
        Command::Edit(args) => nav::edit(&App::load(env)?, ui, &args),
        Command::Clone(args) => nav::clone(&App::load(env)?, ui, &args),
        Command::Browse(args) => nav::browse(&App::load(env)?, ui, &args),
        Command::Open(args) => nav::open(&App::load(env)?, ui, &args),
        Command::Pull(args) => sync_cmd::pull(&App::load(env)?, ui, &args),
        Command::Sync(args) => sync_cmd::sync(&App::load(env)?, ui, &args),
        Command::Update(args) => update::run(&App::load(env)?, ui, &args),
        Command::Favorite(args) => favorite::run(&App::load(env)?, ui, &args),
        Command::Setup(args) => setup::run(App::load(env)?, ui, &args),
        Command::Auth(args) => match args.action {
            AuthAction::Login(arg) => auth_cmd::login(&gh::ProcessGh, ui, &arg.domain),
            AuthAction::Logout(arg) => auth_cmd::logout(&gh::ProcessGh, ui, &arg.domain),
            AuthAction::Status => auth_cmd::status(&App::load(env)?, ui),
        },
    }?;
    ui.flush()?;
    Ok(code)
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
