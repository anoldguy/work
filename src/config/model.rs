//! The typed configuration and its invariants (pure).

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::config::{ConfigError, ExpandedPath, Locations};
use crate::domain::{ApiUrl, Domain, Org};

/// The domain used when none is configured, so `work cd foo` works before
/// `work setup` has run.
pub const IMPLICIT_DOMAIN: &str = "github.com";

/// `config.toml` exactly as written; every key optional and unknown keys
/// ignored. Validated into a [`Config`] by [`Config::from_file`].
#[derive(Debug, Default, Deserialize)]
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
pub struct DomainConfig {
    /// Repos on the default domain omit the domain from their path.
    #[serde(default)]
    pub is_default: bool,
    /// When non-empty, only these orgs are fetched (unless `--all-orgs`).
    #[serde(default)]
    pub favorite_orgs: Vec<Org>,
    /// Orgs fetched in addition to the user's memberships.
    #[serde(default)]
    pub extra_orgs: Vec<Org>,
    /// REST API base override; computed by `github::api_base` when absent.
    #[serde(default)]
    pub api_url: Option<ApiUrl>,
}

/// The validated configuration.
///
/// Invariants: paths are absolute; at most one domain is the default, and
/// `default` agrees with `domains` (the `is_default` one, or
/// [`IMPLICIT_DOMAIN`] when no domains are configured). `domains` holds only
/// what the file configures; the implicit `github.com` appears through
/// [`Config::effective_domains`] and [`Config::default_domain`], never in
/// `domains`.
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
        let _ = (file, locations, path);
        unimplemented!("W2.4")
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
        let _ = self;
        unimplemented!("W2.4")
    }

    /// The default domain: the one marked `is_default`, or
    /// [`IMPLICIT_DOMAIN`] when no domains are configured, else `None`.
    /// Resolved once by [`Config::from_file`].
    #[must_use]
    pub const fn default_domain(&self) -> Option<&Domain> {
        self.default.as_ref()
    }

    /// The configured `api_url` overrides, for `HttpGitHub::new`.
    #[must_use]
    pub fn api_bases(&self) -> BTreeMap<Domain, ApiUrl> {
        let _ = self;
        unimplemented!("W2.4")
    }
}
