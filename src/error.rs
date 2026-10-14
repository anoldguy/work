//! The top-level error and how it is rendered.
//!
//! `to_string()` is always one summary line. A list error (ambiguous query,
//! failed sync repos, unknown style) also carries items, printed by `main`
//! as `  - <item>` lines after `work: <summary>`; those items come from data,
//! never from `\n` in a message.

use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::error::{ContextKind, ContextValue, ErrorKind};

use crate::auth::AuthError;
use crate::cache::CacheError;
use crate::commands::resolve::ResolveError;
use crate::config::ConfigError;
use crate::domain::NameError;
use crate::gh::GhError;
use crate::git::GitError;
use crate::github::ApiError;
use crate::platform::PlatformError;
use crate::shell::ShellError;
use crate::sync::SyncError;
use crate::ui::UiError;

/// Every failure a command can return. Each module's error folds in through
/// `From`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An invalid value.
    #[error(transparent)]
    Name(#[from] NameError),
    /// Configuration.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// The repository cache.
    #[error(transparent)]
    Cache(#[from] CacheError),
    /// Tokens.
    #[error(transparent)]
    Auth(#[from] AuthError),
    /// The gh CLI.
    #[error(transparent)]
    Gh(#[from] GhError),
    /// git.
    #[error(transparent)]
    Git(#[from] GitError),
    /// The GitHub API.
    #[error(transparent)]
    Api(#[from] ApiError),
    /// Query resolution.
    #[error(transparent)]
    Resolve(#[from] ResolveError),
    /// `pull`/`sync`.
    #[error(transparent)]
    Sync(#[from] SyncError),
    /// Shell integration and `about` styles.
    #[error(transparent)]
    Shell(#[from] ShellError),
    /// The terminal.
    #[error(transparent)]
    Ui(#[from] UiError),
    /// External programs.
    #[error(transparent)]
    Platform(#[from] PlatformError),
    /// A command-line usage error: clap's first line plus
    /// `(see 'work --help')`. Exit code 2.
    #[error("{0}")]
    Usage(String),
    /// Creating a directory (the work directory, or a clone's parent)
    /// failed.
    #[error("cannot create directory {path:?}: {source}")]
    CreateDir {
        /// The directory.
        path: PathBuf,
        /// The I/O error.
        source: io::Error,
    },
}

impl Error {
    /// Reduces a clap usage error to [`Error::Usage`]: one line built from
    /// clap's structured context (so a newline inside an argument cannot cut
    /// it short), with control characters escaped (clap echoes argv), the
    /// valid values or suggestion when clap has one, and
    /// `(see 'work --help')`. Not for `DisplayHelp`/
    /// `DisplayVersion`, which `lib::run` prints normally.
    #[must_use]
    pub fn from_clap(err: &clap::Error) -> Self {
        Self::from_clap_in(err, "work")
    }

    /// [`Error::from_clap`] with the help hint pointing at `command_path`
    /// (`work` or e.g. `work completions`, the deepest subcommand the
    /// arguments reached).
    #[must_use]
    pub fn from_clap_in(err: &clap::Error, command_path: &str) -> Self {
        let arg = || context(err, ContextKind::InvalidArg).unwrap_or_default();
        let value = || context(err, ContextKind::InvalidValue).unwrap_or_default();
        let mut summary = match err.kind() {
            ErrorKind::UnknownArgument => format!("unexpected argument '{}' found", arg()),
            ErrorKind::InvalidSubcommand => format!(
                "unrecognized subcommand '{}'",
                context(err, ContextKind::InvalidSubcommand).unwrap_or_default()
            ),
            ErrorKind::InvalidValue => format!("invalid value '{}' for '{}'", value(), arg()),
            ErrorKind::ValueValidation => {
                let reason = std::error::Error::source(err)
                    .map(ToString::to_string)
                    .unwrap_or_default();
                format!("invalid value '{}' for '{}': {reason}", value(), arg())
            }
            ErrorKind::MissingRequiredArgument => {
                format!("missing required argument {}", arg())
            }
            ErrorKind::MissingSubcommand | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
                "a subcommand is required".to_owned()
            }
            ErrorKind::TooManyValues | ErrorKind::WrongNumberOfValues => {
                format!("unexpected value '{}' for '{}'", value(), arg())
            }
            _ => first_line(&err.to_string()),
        };
        summary = escape_controls(summary.trim_end_matches(':'));

        let mut hints = Vec::new();
        if let Some(valid) = context(err, ContextKind::ValidValue) {
            hints.push(format!("possible values: {valid}"));
        }
        if let Some(similar) = context(err, ContextKind::SuggestedSubcommand)
            .or_else(|| context(err, ContextKind::SuggestedArg))
            .or_else(|| context(err, ContextKind::SuggestedValue))
        {
            hints.push(format!("did you mean '{similar}'?"));
        }
        if !hints.is_empty() {
            summary = format!("{summary} ({})", escape_controls(&hints.join("; ")));
        }
        Self::Usage(format!(
            "{summary} (see '{} --help')",
            escape_controls(command_path)
        ))
    }

    /// Items to print as `  - <item>` lines after the summary; empty for
    /// ordinary errors. Each item is one line: control characters (from
    /// untrusted paths) are escaped.
    #[must_use]
    pub fn list_items(&self) -> Vec<String> {
        match self {
            Self::Resolve(ResolveError::Ambiguous { candidates, .. }) => candidates
                .iter()
                .map(|slug| escape_controls(&slug.to_string()))
                .collect(),
            Self::Sync(SyncError::SomeFailed { failed, .. }) => failed
                .iter()
                .map(|path| escape_controls(&path.to_string_lossy()))
                .collect(),
            Self::Shell(ShellError::UnknownStyle { valid, .. }) => {
                valid.iter().map(|name| escape_controls(name)).collect()
            }
            _ => Vec::new(),
        }
    }

    /// The one-line summary `main` prints after `work: `: `to_string()` with
    /// control characters escaped, so a message that embeds untrusted text
    /// can never span lines whatever its variant.
    #[must_use]
    pub fn summary(&self) -> String {
        escape_controls(&self.to_string())
    }

    /// The process exit code: 0 for a broken stdout pipe, 2 for usage
    /// errors, 130 for Ctrl-C, otherwise 1.
    #[must_use]
    pub fn exit_code(&self) -> ExitCode {
        match self {
            Self::Ui(UiError::BrokenPipe) => ExitCode::SUCCESS,
            Self::Usage(_) => ExitCode::from(2),
            Self::Ui(UiError::Interrupted) => ExitCode::from(130),
            _ => ExitCode::FAILURE,
        }
    }

    /// True when nothing should be printed (a broken stdout pipe).
    #[must_use]
    pub const fn is_silent(&self) -> bool {
        matches!(self, Self::Ui(UiError::BrokenPipe))
    }
}

/// The text of one clap context entry, values joined with `, `.
fn context(err: &clap::Error, kind: ContextKind) -> Option<String> {
    match err.get(kind)? {
        ContextValue::String(text) => Some(text.clone()),
        ContextValue::Strings(texts) => Some(texts.join(", ")),
        ContextValue::StyledStr(text) => Some(text.to_string()),
        ContextValue::StyledStrs(texts) => Some(
            texts
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", "),
        ),
        ContextValue::Number(n) => Some(n.to_string()),
        ContextValue::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// First non-empty line of clap's rendering, without the `error: ` prefix.
fn first_line(rendered: &str) -> String {
    let line = rendered
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("invalid usage");
    line.strip_prefix("error: ").unwrap_or(line).to_owned()
}

/// `s` with every control character replaced by its `\u{..}`/`\n` escape,
/// so it cannot break a line.
fn escape_controls(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_control() {
                c.escape_default().to_string()
            } else {
                c.to_string()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::process::ExitCode;

    use super::Error;
    use crate::shell::ShellError;
    use crate::sync::SyncError;
    use crate::ui::UiError;

    #[test]
    fn list_errors_keep_items_out_of_the_summary() {
        let err = Error::from(SyncError::SomeFailed {
            failed: vec![PathBuf::from("acme/foo"), PathBuf::from("corp/bar\nbaz")],
            total: 40,
        });
        assert_eq!(err.to_string(), "2 of 40 repositories failed to sync");
        assert_eq!(err.list_items(), ["acme/foo", "corp/bar\\nbaz"]);

        let err = Error::from(ShellError::UnknownStyle {
            given: "x\ny".to_owned(),
            valid: vec!["classic", "fire"],
        });
        assert!(!err.to_string().contains('\n'), "{err}");
        assert_eq!(err.list_items(), ["classic", "fire"]);
    }

    fn usage_text(args: &[&str]) -> String {
        use clap::Parser;

        use crate::cli::Cli;

        let err = Cli::try_parse_from(args).expect_err("should be a usage error");
        let usage = Error::from_clap_in(&err, &crate::help_path(args));
        assert_eq!(usage.exit_code(), ExitCode::from(2));
        let text = usage.to_string();
        assert!(!text.contains('\n'), "{text:?}");
        assert!(!text.chars().any(char::is_control), "{text:?}");
        assert!(text.contains("(see 'work"), "{text}");
        text
    }

    #[test]
    fn clap_errors_become_one_usage_line() {
        let text = usage_text(&["work", "--bad\u{1b}flag"]);
        assert!(text.starts_with("unexpected argument"), "{text}");
        assert!(text.ends_with("(see 'work --help')"), "{text}");

        usage_text(&["work", "cd", "a", "b"]);

        let text = usage_text(&["work", "pull"]);
        assert!(text.contains("<QUERY>"), "{text}");
        assert!(text.ends_with("(see 'work pull --help')"), "{text}");
    }

    #[test]
    fn newlines_in_arguments_do_not_cut_the_message() {
        let text = usage_text(&["work", "fr\nob"]);
        assert!(text.contains("fr\\nob"), "{text}");
        let text = usage_text(&["work", "auth", "login", "--domain", "a\nb"]);
        assert!(text.contains("a\\nb"), "{text}");
    }

    #[test]
    fn hints_survive_in_the_one_line() {
        let text = usage_text(&["work", "completions", "nosh"]);
        assert!(text.contains("possible values: bash, zsh, fish"), "{text}");
        assert!(text.ends_with("(see 'work completions --help')"), "{text}");
        let text = usage_text(&["work", "cdd"]);
        assert!(text.contains("did you mean 'cd'?"), "{text}");
    }

    #[test]
    fn a_group_without_a_subcommand_is_a_usage_error_not_help_text() {
        let text = usage_text(&["work", "auth"]);
        assert!(text.starts_with("a subcommand is required"), "{text}");
        assert!(text.ends_with("(see 'work auth --help')"), "{text}");
    }

    #[test]
    fn summary_escapes_whatever_a_variant_embeds() {
        let err = Error::Usage("a\nb".to_owned());
        assert_eq!(err.summary(), "a\\nb");
    }

    #[test]
    fn ordinary_errors_have_no_items() {
        assert_eq!(
            Error::Usage("bad flag".to_owned()).list_items(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn exit_codes_follow_the_table() {
        assert_eq!(
            Error::from(UiError::BrokenPipe).exit_code(),
            ExitCode::SUCCESS
        );
        assert_eq!(Error::Usage("x".to_owned()).exit_code(), ExitCode::from(2));
        assert_eq!(
            Error::from(UiError::Interrupted).exit_code(),
            ExitCode::from(130)
        );
        let not_tty = UiError::NotInteractive { what: "setup" };
        assert_eq!(Error::from(not_tty).exit_code(), ExitCode::FAILURE);
        assert!(Error::from(UiError::BrokenPipe).is_silent());
        assert!(!Error::from(UiError::Interrupted).is_silent());
    }
}
