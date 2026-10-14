//! Configuration: XDG locations, the typed model, and the TOML file.
//!
//! Reads parse the file once into a `toml_edit::DocumentMut` and deserialise
//! it into [`ConfigFile`] with `toml_edit::de::from_document`; edits go
//! through [`ConfigDoc`], a `DocumentMut` that keeps the user's comments and
//! layout.

pub mod locations;
pub mod model;
pub mod store;

pub use locations::{ExpandedPath, Locations};
pub use model::{Config, ConfigFile, DomainConfig, Filters, IMPLICIT_DOMAIN};
pub use store::ConfigDoc;

use std::io;
use std::path::PathBuf;

use crate::domain::Domain;

/// A configuration failure. Paths render with `{:?}` so the message stays on
/// one line whatever the path contains.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// `HOME` is unset, so no default location can be computed.
    #[error("cannot determine the home directory (set HOME)")]
    NoHome,
    /// The config file exists but cannot be read.
    #[error("cannot read config {path:?}: {source}")]
    Read {
        /// The config file.
        path: PathBuf,
        /// The I/O error.
        source: io::Error,
    },
    /// Not valid TOML, or values of the wrong type or format.
    #[error("invalid config {path:?}: {detail}")]
    Parse {
        /// The config file.
        path: PathBuf,
        /// `toml_edit`'s error reduced to one line.
        detail: String,
    },
    /// `[domains.github.com]` written without quotes, which TOML reads as
    /// nested tables.
    #[error(
        "invalid config {path:?}: domain {host:?} is split into nested tables (quote the hostname: [domains.{host:?}])"
    )]
    NestedDomainTable {
        /// The config file.
        path: PathBuf,
        /// The dotted key as written, e.g. `github.com`.
        host: String,
    },
    /// More than one domain has `is_default = true`.
    #[error("invalid config {path:?}: more than one default domain ({first} and {second})")]
    MultipleDefaults {
        /// The config file.
        path: PathBuf,
        /// One default domain.
        first: Domain,
        /// Another default domain.
        second: Domain,
    },
    /// `work_dir` or `cache_path` cannot be used as a directory.
    #[error("invalid {key} {given:?} ({reason})")]
    InvalidPath {
        /// `work_dir` or `cache_path`.
        key: &'static str,
        /// The value as written.
        given: String,
        /// What is wrong with it, e.g. "must be absolute or start with ~/".
        reason: &'static str,
    },
    /// A command named a domain that has no `[domains."<host>"]` table.
    #[error("domain {domain} is not configured (run 'work setup {domain}')")]
    UnknownDomain {
        /// The domain asked for.
        domain: Domain,
    },
    /// A command needs at least one configured domain.
    #[error("no domains are configured (run 'work setup')")]
    NoDomains,
    /// The document has a shape an edit cannot work with, e.g. `domains` is
    /// not a table.
    #[error("cannot edit config {path:?}: {detail}")]
    Edit {
        /// The config file.
        path: PathBuf,
        /// What is in the way.
        detail: String,
    },
    /// Writing the config (or creating its directory) failed.
    #[error("cannot write config {path:?}: {source}")]
    Write {
        /// The file or directory being written.
        path: PathBuf,
        /// The I/O error.
        source: io::Error,
    },
}
