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

    /// The file this document saves to (tests).
    #[cfg(test)]
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

    /// Writes the document atomically (temp file in the same directory,
    /// fsync, rename), creating the directory if needed.
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

    const README_EXAMPLE: &str = r#"work_dir = "~/code"
cache_path = "~/.local/state/work"

[filters]
exclude_orgs = ["that-org-from-2019-you-forgot-to-leave"]
include_orgs = ["my-team"]

[domains."github.com"]
is_default = true
favorite_orgs = ["my-team", "my-company"]

[domains."github.enterprise.com"]
favorite_orgs = ["team-a", "team-b"]
extra_orgs = ["partner-team", "shared-infra"]
"#;

    const COMMENTED: &str = r#"# my config
work_dir = "~/code" # where things live

[domains."a.com"]
# the main one
is_default = true # keep
favorite_orgs = ["x"] # favs

[domains."b.example.com"]
extra_orgs = ["y"]
"#;

    fn setup(text: Option<&str>) -> (tempfile::TempDir, Locations) {
        let dir = tempfile::tempdir().expect("tempdir");
        let env = EnvSnapshot {
            home: Some(dir.path().to_path_buf()),
            ..EnvSnapshot::default()
        };
        let locations = Locations::from_env(&env).expect("locations");
        if let Some(text) = text {
            let parent = locations.config_file.parent().expect("parent");
            fs::create_dir_all(parent).expect("mkdir");
            fs::write(&locations.config_file, text).expect("write");
        }
        (dir, locations)
    }

    fn domain(s: &str) -> Domain {
        s.parse().expect("domain")
    }

    fn org(s: &str) -> Org {
        s.parse().expect("org")
    }

    fn doc_of(text: &str) -> (tempfile::TempDir, ConfigDoc) {
        let (dir, locations) = setup(Some(text));
        let doc = ConfigDoc::open(&locations.config_file).expect("open");
        (dir, doc)
    }

    #[test]
    fn readme_example_loads_in_one_parse() {
        let (_dir, locations) = setup(Some(README_EXAMPLE));
        let config = load(&locations).expect("load");
        assert_eq!(config.default_domain(), Some(&domain("github.com")));
        assert_eq!(config.domains().len(), 2);
        assert_eq!(config.filters().include_orgs, vec![org("my-team")]);
    }

    #[test]
    fn missing_file_gives_implicit_github() {
        let (_dir, locations) = setup(None);
        let config = load(&locations).expect("load");
        assert_eq!(config.effective_domains(), vec![domain("github.com")]);
        assert_eq!(config.default_domain(), Some(&domain("github.com")));
    }

    #[test]
    fn no_domains_gives_implicit_github() {
        let (_dir, locations) = setup(Some("work_dir = \"~/src\"\n"));
        let config = load(&locations).expect("load");
        assert_eq!(config.effective_domains(), vec![domain("github.com")]);
        assert!(config.work_dir().ends_with("src"));
    }

    #[test]
    fn two_defaults_are_rejected() {
        let (_dir, locations) = setup(Some(
            "[domains.\"a.com\"]\nis_default = true\n[domains.\"b.com\"]\nis_default = true\n",
        ));
        assert_matches!(load(&locations), Err(ConfigError::MultipleDefaults { .. }));
    }

    #[test]
    fn uppercase_domain_key_is_rejected_with_a_hint() {
        let (_dir, locations) = setup(Some("[domains.\"GitHub.com\"]\nis_default = true\n"));
        let err = load(&locations).expect_err("uppercase");
        assert_matches!(&err, ConfigError::DomainCase { given, .. } if given == "GitHub.com");
        assert!(err.to_string().contains("github.com"), "{err}");
    }

    #[test]
    fn unquoted_dotted_domain_suggests_quoting() {
        let (_dir, locations) = setup(Some("[domains.github.com]\nis_default = true\n"));
        let err = load(&locations).expect_err("nested");
        assert_matches!(&err, ConfigError::NestedDomainTable { host, .. } if host == "github.com");
        let msg = err.to_string();
        assert!(msg.contains("quote the hostname"), "{msg}");
        assert!(msg.contains("[domains.\"github.com\"]"), "{msg}");
    }

    #[test]
    fn unquoted_three_label_domain_is_detected() {
        let (_dir, locations) = setup(Some("[domains.github.enterprise.com]\n"));
        assert_matches!(
            load(&locations),
            Err(ConfigError::NestedDomainTable { host, .. }) if host == "github.enterprise.com"
        );
    }

    #[test]
    fn quoted_domain_with_subtable_is_not_advised_to_requote() {
        let (_dir, locations) = setup(Some("[domains.\"github.com\".extra]\nx = 1\n"));
        let err = load(&locations).expect_err("subtable");
        assert_matches!(&err, ConfigError::Parse { detail, .. } if detail.contains("sub-table"));
    }

    #[test]
    fn misspelt_keys_are_rejected_with_the_key_named() {
        for text in [
            "WORK_DIR = \"/x\"\n",
            "[filters]\ninclude_org = []\n",
            "[domains.\"a.com\"]\nfavourite_orgs = []\n",
        ] {
            let (_dir, locations) = setup(Some(text));
            let err = load(&locations).expect_err("unknown key");
            assert_matches!(&err, ConfigError::Parse { detail, .. } if detail.contains("unknown field"));
            assert!(!err.to_string().contains('\n'), "{err:?}");
        }
    }

    #[test]
    fn toml_syntax_error_is_one_line() {
        let (_dir, locations) = setup(Some("work_dir = \n[filters\nx = 1\n"));
        let err = load(&locations).expect_err("syntax");
        assert_matches!(&err, ConfigError::Parse { .. });
        assert!(!err.to_string().contains('\n'), "{err:?}");
    }

    #[test]
    fn wrong_type_error_is_one_line() {
        let (_dir, locations) = setup(Some("work_dir = 5\n"));
        let err = load(&locations).expect_err("type");
        assert_matches!(&err, ConfigError::Parse { .. });
        assert!(!err.to_string().contains('\n'), "{err:?}");
    }

    #[test]
    fn set_default_moves_the_flag_and_keeps_comments() {
        let (_dir, mut doc) = doc_of(COMMENTED);
        doc.set_default(&domain("b.example.com")).expect("set");
        insta::assert_snapshot!(doc.doc.to_string());
    }

    #[test]
    fn set_default_unknown_domain_changes_nothing() {
        let (_dir, mut doc) = doc_of(COMMENTED);
        let err = doc.set_default(&domain("nope.com")).expect_err("unknown");
        assert_matches!(err, ConfigError::UnknownDomain { .. });
        assert_eq!(doc.doc.to_string(), COMMENTED);
    }

    #[test]
    fn set_favorites_replaces_and_empty_removes() {
        let (_dir, mut doc) = doc_of(COMMENTED);
        doc.set_favorites(&domain("a.com"), &[org("one"), org("two")])
            .expect("set");
        doc.set_favorites(&domain("b.example.com"), &[org("z")])
            .expect("set");
        insta::assert_snapshot!("set_favorites_replaced", doc.doc.to_string());
        doc.set_favorites(&domain("a.com"), &[]).expect("clear");
        insta::assert_snapshot!("set_favorites_cleared", doc.doc.to_string());
    }

    #[test]
    fn add_domain_to_empty_document() {
        let (_dir, mut doc) = doc_of("");
        doc.add_domain(&domain("github.com"), true).expect("add");
        insta::assert_snapshot!(doc.doc.to_string());
    }

    #[test]
    fn add_domain_keeps_existing_and_can_take_default() {
        let (_dir, mut doc) = doc_of(COMMENTED);
        doc.add_domain(&domain("a.com"), false).expect("existing");
        assert_eq!(doc.doc.to_string(), COMMENTED);
        doc.add_domain(&domain("c.com"), true).expect("new");
        insta::assert_snapshot!(doc.doc.to_string());
    }

    #[test]
    fn set_work_dir_replaces_value_and_keeps_comment() {
        let (_dir, mut doc) = doc_of(COMMENTED);
        doc.set_work_dir("~/src").expect("set");
        insta::assert_snapshot!("set_work_dir_replaced", doc.doc.to_string());
        let (_dir, mut doc) = doc_of("[domains.\"a.com\"]\n");
        doc.set_work_dir("~/src").expect("set");
        insta::assert_snapshot!("set_work_dir_added", doc.doc.to_string());
    }

    #[test]
    fn domains_that_is_not_a_table_is_an_edit_error() {
        let (_dir, mut doc) = doc_of("domains = 3\n");
        assert_matches!(
            doc.add_domain(&domain("a.com"), false),
            Err(ConfigError::Edit { .. })
        );
    }

    #[test]
    fn save_creates_directories_and_round_trips() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested").join("work").join("config.toml");
        let mut doc = ConfigDoc::open(&path).expect("open missing");
        doc.add_domain(&domain("github.com"), true).expect("add");
        doc.set_work_dir("~/src").expect("work_dir");
        doc.save().expect("save");
        let saved = fs::read_to_string(&path).expect("read");
        insta::assert_snapshot!(saved);
        let names: Vec<_> = fs::read_dir(path.parent().expect("parent"))
            .expect("ls")
            .map(|e| e.expect("entry").file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("config.toml")]);
        let reopened = ConfigDoc::open(&path).expect("reopen");
        assert_eq!(reopened.doc.to_string(), saved);
    }

    #[cfg(unix)]
    #[test]
    fn save_through_a_symlink_keeps_the_link_and_updates_the_target() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("dotfiles.toml");
        let link = dir.path().join("config.toml");
        fs::write(&target, COMMENTED).expect("write target");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");
        let mut doc = ConfigDoc::open(&link).expect("open");
        doc.set_default(&domain("b.example.com")).expect("set");
        doc.save().expect("save");
        assert!(
            fs::symlink_metadata(&link)
                .expect("meta")
                .file_type()
                .is_symlink()
        );
        let updated = fs::read_to_string(&target).expect("read target");
        assert_eq!(updated, doc.doc.to_string());
        assert_ne!(updated, COMMENTED);
    }

    #[test]
    fn save_replaces_an_existing_file() {
        let (_dir, mut doc) = doc_of(COMMENTED);
        doc.set_favorites(&domain("a.com"), &[]).expect("clear");
        doc.save().expect("save");
        let on_disk = fs::read_to_string(doc.path()).expect("read");
        assert!(!on_disk.contains("favorite_orgs"));
        assert!(on_disk.contains("# my config"));
    }
}
