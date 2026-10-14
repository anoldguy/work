//! Shell integration: detection, quoting, the `sh-*` scripts, activation,
//! and completions.
//!
//! The wrapper scripts in `shell/` are the contract and are embedded
//! unchanged. `sh-*` output is one shell-agnostic line, so it needs no shell
//! detection; detection is for `activate`, `completions` and hints only.

pub mod activate;
pub mod completions;
pub mod detect;
pub mod quote;
pub mod script;

pub use activate::activation_script;
pub use completions::{completion_script, install_path};
pub use detect::detect;
pub use quote::quote;
pub use script::{cd_script, on_script};

use std::fmt;
use std::io;
use std::path::PathBuf;

/// A supported shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Shell {
    /// GNU bash.
    Bash,
    /// zsh.
    Zsh,
    /// fish.
    Fish,
}

impl Shell {
    /// `bash`, `zsh`, or `fish`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Fish => "fish",
        }
    }

    /// The startup file the activation line belongs in.
    #[must_use]
    pub const fn rc_file(self) -> &'static str {
        match self {
            Self::Bash => "~/.bashrc",
            Self::Zsh => "~/.zshrc",
            Self::Fish => "~/.config/fish/config.fish",
        }
    }

    /// The line that loads the wrapper, e.g. `eval "$(work activate bash)"`.
    #[must_use]
    pub const fn activation_line(self) -> &'static str {
        match self {
            Self::Bash => r#"eval "$(work activate bash)""#,
            Self::Zsh => r#"eval "$(work activate zsh)""#,
            Self::Fish => "work activate fish | source",
        }
    }
}

impl fmt::Display for Shell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl From<Shell> for clap_complete::Shell {
    fn from(value: Shell) -> Self {
        match value {
            Shell::Bash => Self::Bash,
            Shell::Zsh => Self::Zsh,
            Shell::Fish => Self::Fish,
        }
    }
}

/// A shell-integration failure.
#[derive(Debug, thiserror::Error)]
pub enum ShellError {
    /// Plain `work cd`/`work on` without the wrapper function.
    #[error("shell integration is not active (add '{}' to {}, then restart your shell)", .shell.activation_line(), .shell.rc_file())]
    NotActive {
        /// The detected shell, for the hint.
        shell: Shell,
    },
    /// A path with a newline cannot survive fish's line splitting.
    #[error("path {path:?} contains a newline, which the shell wrapper cannot handle")]
    NewlineInPath {
        /// The offending path.
        path: PathBuf,
    },
    /// Scripts are text; a non-UTF-8 path cannot be quoted into one.
    #[error("path {path:?} is not valid UTF-8")]
    NonUtf8Path {
        /// The offending path.
        path: PathBuf,
    },
    /// `about --style` with an unknown name. A list error: `valid` is printed
    /// as `  - <name>` lines after the summary.
    #[error("unknown style {given:?} (valid styles are listed below)")]
    UnknownStyle {
        /// The name given.
        given: String,
        /// Every registered style, in registry order.
        valid: Vec<&'static str>,
    },
    /// Writing the completion file failed.
    #[error("cannot install completions to {path:?}: {source}")]
    Install {
        /// The file or directory being written.
        path: PathBuf,
        /// The I/O error.
        source: io::Error,
    },
}
