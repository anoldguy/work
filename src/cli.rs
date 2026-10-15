//! Command-line definitions (clap derive).
//!
//! Every resolving command flattens [`ResolveArgs`] (`<query> [--pick]`);
//! `--pick` is not global. `sh-cd`/`sh-on` and `commands` are hidden: they
//! exist for the shell wrapper. With no subcommand, help goes to stdout.

use std::num::NonZeroUsize;

use clap::{Args, Parser, Subcommand};

use crate::domain::Domain;
use crate::shell::Shell;

/// `work`: navigate and sync your GitHub repositories.
#[derive(Debug, Parser)]
#[command(name = "work", version, about)]
pub struct Cli {
    /// The subcommand; `None` prints help.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Every subcommand.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Jump to a repository (needs shell integration)
    Cd(CdArgs),
    /// Jump to a repository and open it in your editor (needs shell integration)
    On(ResolveArgs),
    /// Open a repository in your editor
    Edit(ResolveArgs),
    /// Clone a repository into the work directory
    Clone(ResolveArgs),
    /// Open a repository's page in the browser
    Browse(ResolveArgs),
    /// Open a repository's directory in the file manager
    Open(ResolveArgs),
    /// Fetch and fast-forward one repository
    Pull(ResolveArgs),
    /// Fetch and fast-forward every cached repository you have cloned
    Sync(SyncArgs),
    /// List cached repositories
    List(ListArgs),
    /// Refresh the repository cache
    Update(UpdateArgs),
    /// Pick favourite organisations for a domain
    Favorite(FavoriteArgs),
    /// Interactive first-time setup
    Setup(SetupArgs),
    /// Show or change configuration
    Config(ConfigArgs),
    /// Manage GitHub authentication through gh
    Auth(AuthArgs),
    /// Print or install shell completions
    Completions(CompletionsArgs),
    /// Print the shell integration script
    Activate(ActivateArgs),
    /// Print the version
    Version,
    /// Show the animated about screen
    About(AboutArgs),
    /// Target directory of `cd`, for the shell wrapper
    #[command(name = "sh-cd", hide = true)]
    ShCd(CdArgs),
    /// Target directory of `on`, for the shell wrapper
    #[command(name = "sh-on", hide = true)]
    ShOn(ResolveArgs),
    /// Editor on the current directory, for the shell wrapper's `on`
    #[command(name = "sh-editor", hide = true)]
    ShEditor,
}

/// `<query> [--pick]`, shared by every command that resolves one repo.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct ResolveArgs {
    /// Repository query: name, org/name, or domain/org/name
    pub query: String,
    /// Always show the picker when several repositories match
    #[arg(long)]
    pub pick: bool,
}

/// `cd [query] [--pick]`: the query is optional (no query → work directory).
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct CdArgs {
    /// Repository query; omit to go to the work directory
    pub query: Option<String>,
    /// Always show the picker when several repositories match
    #[arg(long)]
    pub pick: bool,
}

/// `sync [--jobs N]`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct SyncArgs {
    /// Repositories to sync at once
    #[arg(long, short = 'j', default_value = "8", value_parser = parse_jobs)]
    pub jobs: NonZeroUsize,
}

/// `--jobs`: a whole number of at least 1, with a plain-English complaint
/// (clap's own is "number would be zero for non-zero type").
fn parse_jobs(raw: &str) -> Result<NonZeroUsize, String> {
    let n: usize = raw
        .parse()
        .map_err(|_| "must be a whole number".to_owned())?;
    NonZeroUsize::new(n).ok_or_else(|| "must be at least 1".to_owned())
}

/// `list [query] [--filter]`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct ListArgs {
    /// Rank repositories against this query
    pub query: Option<String>,
    /// Plain substring match on the full slug, without frecency
    #[arg(long)]
    pub filter: bool,
    /// Offline completion candidates (name and org/name); silent on failure
    #[arg(long, hide = true, conflicts_with_all = ["query", "filter"])]
    pub complete: bool,
}

/// `update [--all-orgs]`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct UpdateArgs {
    /// Fetch every org, ignoring favourites
    #[arg(long)]
    pub all_orgs: bool,
}

/// `favorite [domain]`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct FavoriteArgs {
    /// Domain to pick favourites for (asks when several are configured)
    pub domain: Option<Domain>,
}

/// `setup [domain]`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct SetupArgs {
    /// Domain to set up (default: github.com)
    pub domain: Option<Domain>,
}

/// `config [--edit] [path | default [domain]]`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct ConfigArgs {
    /// Open the config file in your editor
    #[arg(long)]
    pub edit: bool,
    /// What to do; omit to show a summary
    #[command(subcommand)]
    pub action: Option<ConfigAction>,
}

/// `config` subcommands.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum ConfigAction {
    /// Show the config file location and whether it exists
    Path,
    /// Set the default domain (asks when omitted)
    Default {
        /// The new default domain
        domain: Option<Domain>,
    },
}

/// `auth <login|logout|status>`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
#[command(arg_required_else_help = false)]
pub struct AuthArgs {
    /// What to do
    #[command(subcommand)]
    pub action: AuthAction,
}

/// `auth` subcommands.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum AuthAction {
    /// Log in with gh
    Login(DomainArg),
    /// Log out with gh
    Logout(DomainArg),
    /// Show token status for every domain
    Status,
}

/// `--domain <domain>`, defaulting to `github.com`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct DomainArg {
    /// GitHub host
    #[arg(long, default_value = "github.com")]
    pub domain: Domain,
}

/// `completions [shell] [--install]`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct CompletionsArgs {
    /// Shell (default: detected from $SHELL)
    pub shell: Option<Shell>,
    /// Write the script to the shell's completion directory
    #[arg(long)]
    pub install: bool,
}

/// `activate [shell]`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct ActivateArgs {
    /// Shell (default: detected from $SHELL)
    pub shell: Option<Shell>,
}

/// `about [--style name]`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct AboutArgs {
    /// Animation style (default: random)
    #[arg(long, hide = true)]
    pub style: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use clap::error::ErrorKind;
    use clap::{CommandFactory, Parser};

    use super::{CdArgs, Cli, Command, ConfigAction};

    #[test]
    fn definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn resolving_commands_take_pick_after_the_query() {
        let cli = Cli::try_parse_from(["work", "sh-cd", "foo", "--pick"]).expect("parses");
        assert_matches!(
            cli.command,
            Some(Command::ShCd(CdArgs { query: Some(ref q), pick: true })) if q == "foo"
        );
    }

    #[test]
    fn list_complete_conflicts_with_a_query_and_filter() {
        for args in [
            &["work", "list", "--complete", "foo"][..],
            &["work", "list", "--complete", "--filter"],
            &["work", "list", "foo", "--filter", "--complete"],
        ] {
            let err = Cli::try_parse_from(args).expect_err("conflict");
            assert_eq!(err.kind(), ErrorKind::ArgumentConflict, "{args:?}");
        }
        let cli = Cli::try_parse_from(["work", "list", "--complete"]).expect("parses");
        assert_matches!(cli.command, Some(Command::List(ref args)) if args.complete);
    }

    #[test]
    fn config_edit_conflicts_with_a_subcommand() {
        assert!(Cli::try_parse_from(["work", "config", "--edit", "path"]).is_err());
        let cli = Cli::try_parse_from(["work", "config", "default"]).expect("parses");
        assert_matches!(
            cli.command,
            Some(Command::Config(ref args)) if args.action == Some(ConfigAction::Default { domain: None })
        );
    }
}
