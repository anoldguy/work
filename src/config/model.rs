//! The typed configuration and its invariants (pure).

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::config::{ConfigError, ExpandedPath, Locations};
use crate::domain::{Domain, Org};

/// The domain used when none is configured.
pub const IMPLICIT_DOMAIN: &str = "github.com";

/// `config.toml` as written. Unknown keys are rejected so a misspelt key is
/// not silently ignored.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    /// Raw `work_dir`, may start with `~`.
    #[serde(default)]
    pub work_dir: Option<String>,
    /// Raw `cache_path`, may start with `~`.
    #[serde(default)]
    pub cache_path: Option<String>,
    /// The `[filters]` table.
    #[serde(default)]
    pub filters: Filters,
    /// The `[domains."<host>"]` tables.
    #[serde(default)]
    pub domains: BTreeMap<Domain, DomainConfig>,
}

/// Global org filters (`[filters]`). `include_orgs` wins when non-empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Filters {
    /// Only these orgs are fetched, when non-empty.
    #[serde(default)]
    pub include_orgs: Vec<Org>,
    /// These orgs are never fetched (ignored when `include_orgs` is set).
    #[serde(default)]
    pub exclude_orgs: Vec<Org>,
}

/// One `[domains."<host>"]` table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DomainConfig {
    /// Repos on the default domain omit the domain from their path.
    #[serde(default)]
    pub is_default: bool,
    /// When non-empty, only these orgs are fetched.
    #[serde(default)]
    pub favorite_orgs: Vec<Org>,
    /// Orgs fetched in addition to the user's memberships.
    #[serde(default)]
    pub extra_orgs: Vec<Org>,
}

/// The validated configuration.
///
/// Invariants: paths are absolute; at most one domain is the default.
/// `domains` holds only what the file configures; the implicit domain appears
/// only through [`Config::effective_domains`] and [`Config::default_domain`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    work_dir: ExpandedPath,
    cache_dir: ExpandedPath,
    filters: Filters,
    domains: BTreeMap<Domain, DomainConfig>,
    default: Option<Domain>,
}

impl Config {
    /// Validates `file`, filling defaults from `locations`. `path` is the
    /// config file, for error messages.
    ///
    /// # Errors
    ///
    /// [`ConfigError::InvalidPath`] for an unusable `work_dir`/`cache_path`,
    /// or [`ConfigError::MultipleDefaults`].
    pub fn from_file(
        file: ConfigFile,
        locations: &Locations,
        path: &Path,
    ) -> Result<Self, ConfigError> {
        let ConfigFile {
            work_dir,
            cache_path,
            filters,
            domains,
        } = file;
        let work_dir = resolve_path(
            work_dir.as_deref(),
            "work_dir",
            &locations.default_work_dir,
            &locations.home,
        )?;
        let cache_dir = resolve_path(
            cache_path.as_deref(),
            "cache_path",
            &locations.default_cache_dir,
            &locations.home,
        )?;
        let mut defaults = domains
            .iter()
            .filter(|(_, cfg)| cfg.is_default)
            .map(|(domain, _)| domain);
        let default = defaults.next().cloned();
        if let (Some(first), Some(second)) = (&default, defaults.next()) {
            return Err(ConfigError::MultipleDefaults {
                path: path.to_path_buf(),
                first: first.clone(),
                second: second.clone(),
            });
        }
        let default = default.or_else(|| domains.is_empty().then(implicit_domain));
        Ok(Self {
            work_dir,
            cache_dir,
            filters,
            domains,
            default,
        })
    }

    /// Root of the clone hierarchy.
    #[must_use]
    pub fn work_dir(&self) -> &Path {
        self.work_dir.as_path()
    }

    /// Directory holding `cache.json`.
    #[must_use]
    pub fn cache_dir(&self) -> &Path {
        self.cache_dir.as_path()
    }

    /// Global org filters.
    #[must_use]
    pub const fn filters(&self) -> &Filters {
        &self.filters
    }

    /// The configured domains (without the implicit one).
    #[must_use]
    pub const fn domains(&self) -> &BTreeMap<Domain, DomainConfig> {
        &self.domains
    }

    /// The domains to fetch and resolve against: the configured ones, or
    /// just [`IMPLICIT_DOMAIN`] when none are configured.
    #[must_use]
    pub fn effective_domains(&self) -> Vec<Domain> {
        if self.domains.is_empty() {
            vec![implicit_domain()]
        } else {
            self.domains.keys().cloned().collect()
        }
    }

    /// The default domain: the one marked `is_default`, or
    /// [`IMPLICIT_DOMAIN`] when no domains are configured, else `None`.
    #[must_use]
    pub const fn default_domain(&self) -> Option<&Domain> {
        self.default.as_ref()
    }
}

/// [`IMPLICIT_DOMAIN`] as a [`Domain`].
fn implicit_domain() -> Domain {
    IMPLICIT_DOMAIN
        .parse()
        .expect("IMPLICIT_DOMAIN is a valid lowercase hostname")
}

/// `raw` expanded against `home`, or `default` (already absolute) when unset.
fn resolve_path(
    raw: Option<&str>,
    key: &'static str,
    default: &Path,
    home: &Path,
) -> Result<ExpandedPath, ConfigError> {
    match raw {
        Some(raw) => ExpandedPath::expand(raw, key, home),
        None => ExpandedPath::from_absolute(default.to_path_buf()).ok_or_else(|| {
            ConfigError::InvalidPath {
                key,
                given: default.display().to_string(),
                reason: "default location is not absolute",
            }
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::env::EnvSnapshot;

    fn locations() -> Locations {
        let env = EnvSnapshot {
            home: Some("/home/u".into()),
            ..EnvSnapshot::default()
        };
        Locations::from_env(&env).expect("locations")
    }

    fn domain(s: &str) -> Domain {
        s.parse().expect("domain")
    }

    fn build(file: ConfigFile) -> Result<Config, ConfigError> {
        Config::from_file(file, &locations(), Path::new("/c.toml"))
    }

    fn with_domains(defaults: &[(&str, bool)]) -> ConfigFile {
        ConfigFile {
            domains: defaults
                .iter()
                .map(|(d, is_default)| {
                    (
                        domain(d),
                        DomainConfig {
                            is_default: *is_default,
                            ..DomainConfig::default()
                        },
                    )
                })
                .collect(),
            ..ConfigFile::default()
        }
    }

    #[test]
    fn empty_file_gets_default_locations_and_implicit_domain() {
        let c = build(ConfigFile::default()).expect("config");
        assert_eq!(c.work_dir(), Path::new("/home/u/code"));
        assert_eq!(c.cache_dir(), Path::new("/home/u/.local/state/work"));
        assert_eq!(c.effective_domains(), vec![domain("github.com")]);
        assert_eq!(c.default_domain(), Some(&domain("github.com")));
        assert!(c.domains().is_empty());
    }

    #[test]
    fn tilde_paths_expand_against_home() {
        let c = build(ConfigFile {
            work_dir: Some("~/src".into()),
            cache_path: Some("/var/cache/w".into()),
            ..ConfigFile::default()
        })
        .expect("config");
        assert_eq!(c.work_dir(), Path::new("/home/u/src"));
        assert_eq!(c.cache_dir(), Path::new("/var/cache/w"));
    }

    #[test]
    fn relative_work_dir_is_rejected() {
        let err = build(ConfigFile {
            work_dir: Some("code".into()),
            ..ConfigFile::default()
        })
        .expect_err("relative");
        assert_matches!(
            err,
            ConfigError::InvalidPath {
                key: "work_dir",
                ..
            }
        );
    }

    #[test]
    fn two_defaults_are_rejected() {
        let err = build(with_domains(&[("a.com", true), ("b.com", true)])).expect_err("two");
        assert_matches!(err, ConfigError::MultipleDefaults { .. });
    }

    #[test]
    fn configured_domains_without_default_have_none() {
        let c = build(with_domains(&[("a.com", false)])).expect("config");
        assert_eq!(c.default_domain(), None);
        assert_eq!(c.effective_domains(), vec![domain("a.com")]);
    }

    #[test]
    fn marked_domain_is_the_default() {
        let c = build(with_domains(&[("a.com", false), ("b.com", true)])).expect("config");
        assert_eq!(c.default_domain(), Some(&domain("b.com")));
    }
}
