//! Loading `config.toml`, and comment-preserving edits to it.

use std::path::{Path, PathBuf};

use crate::config::{Config, ConfigError, Locations};
use crate::domain::{Domain, Org};

/// Loads and validates `locations.config_file`. A missing file yields the
/// defaults (no domains).
///
/// Parses once: text into a `toml_edit::DocumentMut`, the unquoted
/// dotted-domain check on that document, then
/// `toml_edit::de::from_document(doc)` into a `ConfigFile` (which consumes
/// the document; nothing here needs it afterwards).
///
/// # Errors
///
/// [`ConfigError::Read`], [`ConfigError::Parse`],
/// [`ConfigError::NestedDomainTable`], or a validation error from
/// `Config::from_file`.
pub fn load(locations: &Locations) -> Result<Config, ConfigError> {
    let _ = locations;
    unimplemented!("W2.4")
}

/// An editable config document that keeps the user's comments and layout.
///
/// Used by `config default`, `favorite` and `setup`. Edits only touch the
/// keys they name; [`ConfigDoc::save`] writes atomically. It parses the file
/// itself rather than sharing [`load`]'s document: `load` consumes its
/// document when deserialising, and an edit must start from the file as it
/// is on disk now. Domain tables are found by their canonical (lowercase)
/// name; `load` rejects any other spelling.
#[derive(Debug)]
pub struct ConfigDoc {
    path: PathBuf,
    doc: toml_edit::DocumentMut,
}

impl ConfigDoc {
    /// Opens `path`; a missing file gives an empty document.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Read`] or [`ConfigError::Parse`].
    pub fn open(path: &Path) -> Result<Self, ConfigError> {
        let _ = path;
        unimplemented!("W2.4")
    }

    /// The file this document saves to.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Marks `d` as the only default domain (clearing `is_default` on the
    /// others).
    ///
    /// # Errors
    ///
    /// [`ConfigError::UnknownDomain`] when `d` has no table, or
    /// [`ConfigError::Edit`] when the document's shape is in the way.
    pub fn set_default(&mut self, d: &Domain) -> Result<(), ConfigError> {
        let _ = (&mut self.doc, d);
        unimplemented!("W2.4")
    }

    /// Replaces `d`'s `favorite_orgs`; an empty slice removes the key.
    ///
    /// # Errors
    ///
    /// [`ConfigError::UnknownDomain`] or [`ConfigError::Edit`].
    pub fn set_favorites(&mut self, d: &Domain, orgs: &[Org]) -> Result<(), ConfigError> {
        let _ = (self, d, orgs);
        unimplemented!("W2.4")
    }

    /// Adds a `[domains."<d>"]` table if missing; with `make_default`, also
    /// makes it the only default.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Edit`].
    pub fn add_domain(&mut self, d: &Domain, make_default: bool) -> Result<(), ConfigError> {
        let _ = (self, d, make_default);
        unimplemented!("W2.4")
    }

    /// Sets `work_dir` to `raw` as typed (it may start with `~`).
    ///
    /// # Errors
    ///
    /// [`ConfigError::Edit`].
    pub fn set_work_dir(&mut self, raw: &str) -> Result<(), ConfigError> {
        let _ = (self, raw);
        unimplemented!("W2.4")
    }

    /// Writes the document atomically (temp file in the same directory,
    /// fsync, rename), creating the directory if needed.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Write`].
    pub fn save(&self) -> Result<(), ConfigError> {
        let _ = self;
        unimplemented!("W2.4")
    }
}
