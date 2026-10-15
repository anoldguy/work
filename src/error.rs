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

use crate::cache::CacheError;
use crate::commands::resolve::ResolveError;
use crate::config::ConfigError;
use crate::domain::NameError;
use crate::gh::GhError;
use crate::git::GitError;
use crate::platform::PlatformError;
use crate::shell::ShellError;
use crate::sync::SyncError;
use crate::text::one_line;
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
    /// The gh CLI.
    #[error(transparent)]
    Gh(#[from] GhError),
    /// git.
    #[error(transparent)]
    Git(#[from] GitError),
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
    /// it short), made safe by [`one_line`] (clap echoes argv), the
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
        summary = one_line(summary.trim_end_matches(':'));

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
            summary = format!("{summary} ({})", one_line(&hints.join("; ")));
        }
        Self::Usage(format!(
            "{summary} (see '{} --help')",
            one_line(command_path)
        ))
    }

    /// Items to print as `  - <item>` lines after the summary; empty for
    /// ordinary errors. Each item (from untrusted paths) goes through
    /// [`one_line`].
    #[must_use]
    pub fn list_items(&self) -> Vec<String> {
        match self {
            Self::Resolve(ResolveError::Ambiguous { candidates, .. }) => candidates
                .iter()
                .map(|slug| one_line(&slug.to_string()))
                .collect(),
            Self::Sync(SyncError::SomeFailed { failed, .. }) => failed
                .iter()
                .map(|path| one_line(&path.to_string_lossy()))
                .collect(),
            Self::Shell(ShellError::UnknownStyle { valid, .. }) => {
                valid.iter().map(|name| one_line(name)).collect()
            }
            _ => Vec::new(),
        }
    }

    /// The one-line summary `main` prints after `work: `: `to_string()`
    /// through [`one_line`], so a message that embeds untrusted text can
    /// never span lines whatever its variant.
    #[must_use]
    pub fn summary(&self) -> String {
        one_line(&self.to_string())
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
