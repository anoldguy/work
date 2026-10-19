//! The top-level error and how it is rendered.

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
use crate::state_file::StateError;
use crate::sync::SyncError;
use crate::text::one_line;
use crate::ui::UiError;

/// Every failure a command can return.
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
    /// A state file such as `adopted.json`.
    #[error(transparent)]
    State(#[from] StateError),
    /// External programs.
    #[error(transparent)]
    Platform(#[from] PlatformError),
    /// A command-line usage error; exits 2.
    #[error("{0}")]
    Usage(String),
    /// Creating a directory failed.
    #[error("cannot create directory {path:?}: {source}")]
    CreateDir {
        /// The directory.
        path: PathBuf,
        /// The I/O error.
        source: io::Error,
    },
}

impl Error {
    /// Reduces a clap usage error to a one-line [`Error::Usage`]. Not for
    /// `DisplayHelp`/`DisplayVersion`.
    #[must_use]
    pub fn from_clap(err: &clap::Error) -> Self {
        Self::from_clap_in(err, "work")
    }

    /// [`Error::from_clap`] with the help hint naming `command_path`.
    #[must_use]
    pub fn from_clap_in(err: &clap::Error, command_path: &str) -> Self {
        let arg = || context(err, ContextKind::InvalidArg).unwrap_or_default();
        let value = || context(err, ContextKind::InvalidValue).unwrap_or_default();
        // Built from clap's structured context rather than its rendering, so
        // a newline inside an argument cannot cut the summary short.
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

    /// Items to print one per line after the summary; empty for ordinary
    /// errors.
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

    /// The message as one line, even when it embeds untrusted text.
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

fn first_line(rendered: &str) -> String {
    let line = rendered
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("invalid usage");
    line.strip_prefix("error: ").unwrap_or(line).to_owned()
}
