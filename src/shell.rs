//! Shell integration: detection, the `sh-*` path line, activation, and
//! completions.
//!
//! The wrapper scripts in `shell/` are the contract and are embedded
//! unchanged. `sh-cd`/`sh-on` print data (one absolute path), never shell
//! code, so they need no shell detection; detection is for `activate`,
//! `completions` and hints only.

pub mod activate;
pub mod completions;
pub mod detect;

pub use activate::activation_script;
pub use completions::{completion_script, install_path};
pub use detect::detect;

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// The line `sh-cd`/`sh-on` print for `path`: the path made absolute
/// (against the current directory, without touching the file system).
///
/// # Errors
///
/// [`ShellError::NonUtf8Path`] or [`ShellError::NewlineInPath`], since the
/// wrapper reads exactly one line of text; [`ShellError::Absolute`] when
/// the current directory is needed and unavailable.
pub fn path_line(path: &Path) -> Result<String, ShellError> {
    let absolute = std::path::absolute(path).map_err(|source| ShellError::Absolute {
        path: path.to_path_buf(),
        source,
    })?;
    let Some(line) = absolute.to_str() else {
        return Err(ShellError::NonUtf8Path { path: absolute });
    };
    if line.contains(['\n', '\r']) {
        return Err(ShellError::NewlineInPath { path: absolute });
    }
    Ok(line.to_owned())
}

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
    /// The wrapper reads text; a non-UTF-8 path cannot be printed as one.
    #[error("path {path:?} is not valid UTF-8")]
    NonUtf8Path {
        /// The offending path.
        path: PathBuf,
    },
    /// A relative path could not be made absolute.
    #[error("cannot make {path:?} absolute: {source}")]
    Absolute {
        /// The relative path.
        path: PathBuf,
        /// Why the current directory is unavailable.
        source: io::Error,
    },
    /// `sh-editor` could not determine the current directory.
    #[error("cannot read the current directory: {source}")]
    CurrentDir {
        /// The I/O error.
        source: io::Error,
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

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::path::Path;

    use super::{ShellError, path_line};

    #[test]
    fn relative_paths_print_absolute() {
        let line = path_line(Path::new("-x/y")).expect("printable");
        assert!(Path::new(&line).is_absolute(), "{line}");
        assert!(line.ends_with("/-x/y"), "{line}");
    }

    #[test]
    fn non_utf8_paths_are_rejected() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let path = Path::new(OsStr::from_bytes(b"/a\xffb"));
        assert_matches!(path_line(path), Err(ShellError::NonUtf8Path { .. }));
    }
}
