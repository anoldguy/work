//! Loading `config.toml`, and comment-preserving edits to it.

use std::fs;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};

use toml_edit::{Array, DocumentMut, Item, Table, TableLike, Value};

use crate::config::{Config, ConfigError, ConfigFile, Locations};
use crate::domain::{Domain, Org};

/// Loads and validates `locations.config_file`. A missing file yields the
/// defaults (no domains).
///
/// # Errors
///
/// [`ConfigError::Read`], [`ConfigError::Parse`],
/// [`ConfigError::NestedDomainTable`], [`ConfigError::DomainCase`], or a
/// validation error.
pub fn load(locations: &Locations) -> Result<Config, ConfigError> {
    let path = &locations.config_file;
    let file = match read_to_string(path)? {
        None => ConfigFile::default(),
        Some(text) => {
            let doc = parse_document(&text, path)?;
            check_domain_keys(&doc, path)?;
            toml_edit::de::from_document(doc).map_err(|err| ConfigError::Parse {
                path: path.clone(),
                detail: with_location(err.message(), err.span(), &text),
            })?
        }
    };
    Config::from_file(file, locations, path)
}

/// The file's text, or `None` when it does not exist.
fn read_to_string(path: &Path) -> Result<Option<String>, ConfigError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(ConfigError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn parse_document(text: &str, path: &Path) -> Result<DocumentMut, ConfigError> {
    text.parse::<DocumentMut>()
        .map_err(|err| ConfigError::Parse {
            path: path.to_path_buf(),
            detail: with_location(err.message(), err.span(), text),
        })
}

/// A TOML error reduced to one line: the message, then the 1-based line and
/// column of `span` in `text` when known.
fn with_location(message: &str, span: Option<Range<usize>>, text: &str) -> String {
    let message = crate::text::one_line(message);
    let Some(start) = span.map(|s| s.start.min(text.len())) else {
        return message;
    };
    let before = text.get(..start).unwrap_or_default();
    let line = before.matches('\n').count().saturating_add(1);
    let column = before
        .rsplit('\n')
        .next()
        .map_or(0, |l| l.chars().count())
        .saturating_add(1);
    format!("{message} (line {line}, column {column})")
}

/// Rejects `[domains.github.com]` written without quotes (TOML reads it as
/// nested tables) and domain keys that are not lowercase.
fn check_domain_keys(doc: &DocumentMut, path: &Path) -> Result<(), ConfigError> {
    let Some(domains) = doc.get("domains").and_then(Item::as_table_like) else {
        return Ok(());
    };
    for (key, item) in domains.iter() {
        if let Some(top) = item.as_table_like()
            && let Some((child, mut table)) = first_subtable(top)
        {
            let mut host = format!("{key}.{child}");
            let mut quoted = is_quoted(domains, key) || is_quoted(top, child);
            while let Some((child, next)) = first_subtable(table) {
                quoted |= is_quoted(table, child);
                host = format!("{host}.{child}");
                table = next;
            }
            return Err(if quoted {
                ConfigError::Parse {
                    path: path.to_path_buf(),
                    detail: format!("domain {key:?} has an unexpected sub-table ({host:?})"),
                }
            } else {
                ConfigError::NestedDomainTable {
                    path: path.to_path_buf(),
                    host,
                }
            });
        }
        if key.chars().any(|c| c.is_ascii_uppercase()) {
            return Err(ConfigError::DomainCase {
                path: path.to_path_buf(),
                given: key.to_owned(),
            });
        }
    }
    Ok(())
}

/// Whether `key` in `table` was written with quotes.
fn is_quoted(table: &dyn TableLike, key: &str) -> bool {
    table.get_key_value(key).is_some_and(|(k, _)| {
        let raw = k.display_repr();
        raw.starts_with('"') || raw.starts_with('\'')
    })
}

/// The first child of `table` that is itself a table.
fn first_subtable(table: &dyn TableLike) -> Option<(&str, &dyn TableLike)> {
    table
        .iter()
        .find_map(|(k, v)| v.as_table_like().map(|t| (k, t)))
}

/// An editable config document that keeps the user's comments and layout.
/// Edits touch only the keys they name; domain tables are found by their
/// lowercase name.
#[derive(Debug)]
pub struct ConfigDoc {
    path: PathBuf,
    doc: DocumentMut,
}

impl ConfigDoc {
    /// Opens `path`; a missing file gives an empty document.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Read`] or [`ConfigError::Parse`].
    pub fn open(path: &Path) -> Result<Self, ConfigError> {
        let text = read_to_string(path)?.unwrap_or_default();
        let doc = parse_document(&text, path)?;
        Ok(Self {
            path: path.to_path_buf(),
            doc,
        })
    }

    /// Marks `d` as the only default domain (clearing `is_default` on the
    /// others).
    ///
    /// # Errors
    ///
    /// [`ConfigError::UnknownDomain`] when `d` has no table, or
    /// [`ConfigError::Edit`] when the document's shape is in the way.
    pub fn set_default(&mut self, d: &Domain) -> Result<(), ConfigError> {
        // Validates first, so a failed edit changes nothing.
        domain_table(&mut self.doc, &self.path, d)?;
        let domains = domains_table(&mut self.doc, &self.path)?;
        for (key, item) in domains.iter_mut() {
            let Some(table) = item.as_table_like_mut() else {
                continue;
            };
            if key.get() == d.as_str() {
                set_value(table, "is_default", true.into());
            } else if table.contains_key("is_default") {
                set_value(table, "is_default", false.into());
            }
        }
        Ok(())
    }

    /// Replaces `d`'s `favorite_orgs`; an empty slice removes the key.
    ///
    /// # Errors
    ///
    /// [`ConfigError::UnknownDomain`] or [`ConfigError::Edit`].
    pub fn set_favorites(&mut self, d: &Domain, orgs: &[Org]) -> Result<(), ConfigError> {
        let table = domain_table(&mut self.doc, &self.path, d)?;
        if orgs.is_empty() {
            table.remove("favorite_orgs");
        } else {
            let array: Array = orgs.iter().map(Org::as_str).collect();
            set_value(table, "favorite_orgs", array.into());
        }
        Ok(())
    }

    /// Adds a `[domains."<d>"]` table if missing; with `make_default`, also
    /// makes it the only default.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Edit`].
    pub fn add_domain(&mut self, d: &Domain, make_default: bool) -> Result<(), ConfigError> {
        if self.doc.get("domains").is_none() {
            let mut implicit = Table::new();
            implicit.set_implicit(true);
            self.doc.insert("domains", Item::Table(implicit));
        }
        let domains = domains_table(&mut self.doc, &self.path)?;
        if !domains.contains_key(d.as_str()) {
            domains.insert(d.as_str(), Item::Table(Table::new()));
        }
        if make_default {
            self.set_default(d)?;
        }
        Ok(())
    }

    /// Sets `work_dir` to `raw` as typed (it may start with `~`).
    ///
    /// # Errors
    ///
    /// [`ConfigError::Edit`].
    pub fn set_work_dir(&mut self, raw: &str) -> Result<(), ConfigError> {
        let root = self.doc.as_table_mut();
        let is_new = !root.contains_key("work_dir");
        set_value(root, "work_dir", raw.into());
        if is_new {
            // Keep a blank line between the new key and the first table.
            let mut first = root.iter_mut().find_map(|(_, item)| item.as_table_mut());
            // An implicit `[domains]` prints no header; its first child does.
            while let Some(table) = first.take() {
                if table.is_implicit() {
                    first = table.iter_mut().find_map(|(_, item)| item.as_table_mut());
                } else {
                    first = Some(table);
                    break;
                }
            }
            if let Some(table) = first
                && table
                    .decor()
                    .prefix()
                    .and_then(|p| p.as_str())
                    .is_none_or(str::is_empty)
            {
                table.decor_mut().set_prefix("\n");
            }
        }
        Ok(())
    }

    /// Writes the document atomically, creating the directory if needed.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Write`].
    pub fn save(&self) -> Result<(), ConfigError> {
        crate::fsutil::atomic_write(&self.path, self.doc.to_string().as_bytes()).map_err(|source| {
            ConfigError::Write {
                path: self.path.clone(),
                source,
            }
        })
    }
}

fn edit_error(path: &Path, detail: &str) -> ConfigError {
    ConfigError::Edit {
        path: path.to_path_buf(),
        detail: detail.to_owned(),
    }
}

/// The `domains` table, which must exist.
fn domains_table<'a>(
    doc: &'a mut DocumentMut,
    path: &Path,
) -> Result<&'a mut dyn TableLike, ConfigError> {
    doc.get_mut("domains")
        .ok_or_else(|| edit_error(path, "there is no [domains] table"))?
        .as_table_like_mut()
        .ok_or_else(|| edit_error(path, "domains is not a table"))
}

/// `d`'s table.
fn domain_table<'a>(
    doc: &'a mut DocumentMut,
    path: &Path,
    d: &Domain,
) -> Result<&'a mut dyn TableLike, ConfigError> {
    let Some(domains) = doc.get_mut("domains") else {
        return Err(ConfigError::UnknownDomain { domain: d.clone() });
    };
    domains
        .as_table_like_mut()
        .ok_or_else(|| edit_error(path, "domains is not a table"))?
        .get_mut(d.as_str())
        .ok_or_else(|| ConfigError::UnknownDomain { domain: d.clone() })?
        .as_table_like_mut()
        .ok_or_else(|| edit_error(path, &format!("domains.{:?} is not a table", d.as_str())))
}

/// Sets `key` to `new`, keeping the old value's surrounding whitespace and
/// trailing comment.
fn set_value(table: &mut dyn TableLike, key: &str, mut new: Value) {
    if let Some(item) = table.get_mut(key) {
        if let Some(old) = item.as_value() {
            *new.decor_mut() = old.decor().clone();
        }
        // Replacing the item in place keeps the key's own leading comments.
        *item = Item::Value(new);
    } else {
        table.insert(key, Item::Value(new));
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::env::EnvSnapshot;

    fn load_text(text: &str) -> Result<Config, ConfigError> {
        let dir = tempfile::tempdir().expect("tempdir");
        let env = EnvSnapshot {
            home: Some(dir.path().to_path_buf()),
            ..EnvSnapshot::default()
        };
        let locations = Locations::from_env(&env).expect("locations");
        fs::create_dir_all(locations.config_file.parent().expect("parent")).expect("mkdir");
        fs::write(&locations.config_file, text).expect("write");
        load(&locations)
    }

    #[test]
    fn the_spec_example_loads() {
        let config = load_text(
            r#"work_dir = "~/code"
cache_path = "~/.local/state/work"

[filters]
exclude_orgs = ["archived-org"]

[domains."github.com"]
is_default = true
favorite_orgs = ["my-team", "my-company"]

[domains."github.enterprise.com"]
favorite_orgs = ["corp-team"]
"#,
        )
        .expect("load");
        assert_eq!(
            config.default_domain(),
            Some(&"github.com".parse().expect("domain"))
        );
        assert_eq!(config.domains().len(), 2);
    }

    #[test]
    fn misspelt_keys_are_rejected() {
        for text in [
            "WORK_DIR = \"/x\"\n",
            "[filters]\ninclude_org = []\n",
            "[domains.\"a.com\"]\nfavourite_orgs = []\n",
        ] {
            assert_matches!(load_text(text), Err(ConfigError::Parse { .. }), "{text}");
        }
    }

    #[test]
    fn unquoted_and_uppercase_domains_are_rejected() {
        assert_matches!(
            load_text("[domains.github.com]\nis_default = true\n"),
            Err(ConfigError::NestedDomainTable { host, .. }) if host == "github.com"
        );
        assert_matches!(
            load_text("[domains.\"GitHub.com\"]\n"),
            Err(ConfigError::DomainCase { .. })
        );
    }
}
