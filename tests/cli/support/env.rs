//! [`TestEnv`]: a hermetic home, shims and seeded state.

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use assert_cmd::Command;
use serde_json::{Map, Value, json};
use tempfile::TempDir;
use toml_edit::{Array, DocumentMut, Item, Table, value};

use super::git::{Origin, git};
use super::shims::{Shim, install_script};

/// A cache timestamp old enough to be stale under any 24 h rule.
const STALE_TIME: &str = "2000-01-01T00:00:00Z";

/// Everything one CLI test needs, rooted in a tempdir that is deleted on
/// drop:
///
/// ```text
/// <root>/home            HOME (cwd of the command)
/// <root>/home/.config    XDG_CONFIG_HOME   -> work/config.toml
/// <root>/home/.local/state  XDG_STATE_HOME -> work/cache.json
/// <root>/home/.local/share  XDG_DATA_HOME
/// <root>/home/code       work_dir
/// <root>/shims           the only PATH entry: gh, editor, opener, git, sh
/// <root>/origins/<org>/<repo>   bare origins from [`TestEnv::origin`]
/// ```
#[derive(Debug)]
pub struct TestEnv {
    root: TempDir,
    token: Option<String>,
    /// The default domain of the last written config; drives `repo_path`.
    default_domain: Mutex<Option<String>>,
    /// The `gh` shim: `set_token(host, ..)` for `gh auth token`/`status`,
    /// `set_api`/`fail_api` for `gh api`.
    pub gh: Shim,
    /// The editor shim, exported as `WORK_EDITOR`.
    pub editor: Shim,
    /// The opener shim: `xdg-open` on Linux, `open` on macOS.
    pub opener: Shim,
}

impl Default for TestEnv {
    fn default() -> Self {
        Self::new()
    }
}

impl TestEnv {
    /// Creates the directories and shims. `GITHUB_TOKEN` defaults
    /// to `test-token`; see [`TestEnv::token`].
    ///
    /// # Panics
    ///
    /// Panics when the tempdir cannot be set up or `git` is not on `PATH`.
    #[must_use]
    pub fn new() -> Self {
        let root = tempfile::tempdir().expect("tempdir");
        let shims = root.path().join("shims");
        fs::create_dir_all(&shims).expect("create shim dir");
        let home = root.path().join("home");
        for dir in [".config", ".local/state", ".local/share", "code"] {
            fs::create_dir_all(home.join(dir)).expect("create home dirs");
        }
        // PATH holds only the shim dir, so expose git and sh inside it.
        install_git_wrapper(&shims.join("git"));
        symlink("/bin/sh", shims.join("sh")).expect("link sh");
        let opener_name = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        Self {
            gh: Shim::install(&shims, "gh"),
            editor: Shim::install(&shims, "editor"),
            opener: Shim::install(&shims, opener_name),
            token: Some("test-token".to_owned()),
            default_domain: Mutex::new(None),
            root,
        }
    }

    /// Sets (or with `None` removes) the `GITHUB_TOKEN` the command sees.
    #[must_use]
    pub fn token(mut self, token: Option<&str>) -> Self {
        self.token = token.map(str::to_owned);
        self
    }

    /// The tempdir root.
    #[must_use]
    pub fn root(&self) -> &Path {
        self.root.path()
    }

    /// `HOME`, also the command's working directory.
    #[must_use]
    pub fn home(&self) -> PathBuf {
        self.root().join("home")
    }

    /// The configured `work_dir` (`<home>/code`).
    #[must_use]
    pub fn work_dir(&self) -> PathBuf {
        self.home().join("code")
    }

    /// The shim directory (the whole `PATH`).
    #[must_use]
    pub fn shim_dir(&self) -> PathBuf {
        self.root().join("shims")
    }

    /// `<XDG_CONFIG_HOME>/work/config.toml`.
    #[must_use]
    pub fn config_path(&self) -> PathBuf {
        self.home().join(".config/work/config.toml")
    }

    /// `<XDG_STATE_HOME>/work/cache.json`.
    #[must_use]
    pub fn cache_path(&self) -> PathBuf {
        self.home().join(".local/state/work/cache.json")
    }

    /// `<XDG_STATE_HOME>/work/frecency.json`.
    #[must_use]
    pub fn frecency_path(&self) -> PathBuf {
        self.home().join(".local/state/work/frecency.json")
    }

    /// Where the clone for `slug` (`domain/org/repo`) lives under
    /// `work_dir`: `<work_dir>/<org>/<repo>` for the default domain of the
    /// config last written with [`ConfigSeed::write`], else
    /// `<work_dir>/<domain>/<org>/<repo>`.
    #[must_use]
    pub fn repo_path(&self, slug: &str) -> PathBuf {
        let default = self
            .default_domain
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let relative = default
            .as_deref()
            .and_then(|domain| slug.strip_prefix(domain)?.strip_prefix('/'))
            .unwrap_or(slug);
        self.work_dir().join(relative)
    }

    /// The complete environment of [`TestEnv::command`], for assertions.
    #[must_use]
    pub fn vars(&self) -> Vec<(&'static str, OsString)> {
        let home = self.home();
        let mut vars: Vec<(&'static str, OsString)> = vec![
            ("HOME", home.clone().into()),
            ("XDG_CONFIG_HOME", home.join(".config").into()),
            ("XDG_STATE_HOME", home.join(".local/state").into()),
            ("XDG_DATA_HOME", home.join(".local/share").into()),
            ("PATH", self.shim_dir().into()),
            ("WORK_EDITOR", self.editor.path().into()),
            ("GIT_CONFIG_GLOBAL", "/dev/null".into()),
            ("GIT_CONFIG_NOSYSTEM", "1".into()),
            ("GIT_AUTHOR_NAME", "Test".into()),
            ("GIT_AUTHOR_EMAIL", "test@example.invalid".into()),
            ("GIT_COMMITTER_NAME", "Test".into()),
            ("GIT_COMMITTER_EMAIL", "test@example.invalid".into()),
        ];
        if let Some(token) = &self.token {
            vars.push(("GITHUB_TOKEN", token.into()));
        }
        vars
    }

    /// A `work` command with a fully controlled environment: `env_clear`,
    /// then exactly [`TestEnv::vars`], running in the test home.
    #[must_use]
    pub fn command(&self) -> Command {
        let mut cmd = super::work();
        cmd.current_dir(self.home());
        for (name, value) in self.vars() {
            cmd.env(name, value);
        }
        cmd
    }

    /// Starts a config to seed; call `.write()` to save it.
    #[must_use]
    pub fn config(&self) -> ConfigSeed<'_> {
        ConfigSeed {
            env: self,
            domains: Vec::new(),
            include_orgs: Vec::new(),
            exclude_orgs: Vec::new(),
        }
    }

    /// Writes `text` verbatim as `config.toml` (for invalid-config tests).
    pub fn write_config_raw(&self, text: &str) {
        write_file(&self.config_path(), text);
    }

    /// Starts a cache to seed (repos, staleness, and the frecency in
    /// `frecency.json`); call `.write()`.
    #[must_use]
    pub fn cache(&self) -> CacheSeed<'_> {
        CacheSeed {
            env: self,
            repos: Vec::new(),
            updated_at: None,
            frecency: Vec::new(),
            query_frecency: Vec::new(),
        }
    }

    /// Seeds a fresh `cache.json` (timestamped now) holding `repos`.
    pub fn seed_cache(&self, repos: &[RepoSeed]) {
        self.cache().repos(repos).write();
    }

    /// Seeds a `cache.json` older than 24 hours.
    pub fn seed_stale_cache(&self, repos: &[RepoSeed]) {
        self.cache().repos(repos).updated_at(STALE_TIME).write();
    }

    /// Seeds `cache.json` with an explicit RFC 3339 `updated_at`.
    pub fn seed_cache_at(&self, repos: &[RepoSeed], updated_at: &str) {
        self.cache().repos(repos).updated_at(updated_at).write();
    }

    /// Creates a bare origin for `org/repo` with one `main` commit.
    pub fn origin(&self, org: &str, repo: &str) -> Origin {
        Origin::create(&self.root().join("origins").join(org).join(repo))
    }

    /// Clones `origin` to `<work_dir>/<slug>` and returns the path.
    pub fn clone_into_work_dir(&self, origin: &Origin, slug: &str) -> PathBuf {
        let dest = self.repo_path(slug);
        origin.clone_to(&dest);
        git(&dest, &["config", "user.name", "Test"]);
        git(&dest, &["config", "user.email", "test@example.invalid"]);
        dest
    }

    /// From now on, every `git` the command runs appends one line to the
    /// returned log: its arguments, space-separated.
    ///
    /// # Panics
    ///
    /// Panics when the shim dir cannot be rewritten.
    #[must_use]
    pub fn record_git_args(&self) -> PathBuf {
        let shim = self.shim_dir().join("git");
        let real = self.shim_dir().join("git-real");
        fs::rename(&shim, &real).expect("move git wrapper aside");
        let log = self.root().join("git-args.log");
        let script = format!(
            "#!/bin/sh\n\
             if [ -n \"$WORK_SHIM_PROBE\" ]; then exit 0; fi\n\
             printf '%s\\n' \"$*\" >> {log}\n\
             exec {real} \"$@\"\n",
            log = sh_quote(&log.display().to_string()),
            real = sh_quote(&real.display().to_string()),
        );
        install_script(&shim, &script);
        log
    }
}

/// One `[domains."<host>"]` section for [`ConfigSeed`].
#[derive(Debug, Clone)]
pub struct DomainCfg {
    host: String,
    is_default: bool,
    favorite_orgs: Vec<String>,
    extra_orgs: Vec<String>,
}

impl DomainCfg {
    /// A domain section for `host`.
    #[must_use]
    pub fn new(host: &str) -> Self {
        Self {
            host: host.to_owned(),
            is_default: false,
            favorite_orgs: Vec::new(),
            extra_orgs: Vec::new(),
        }
    }

    /// Marks the domain `is_default = true`.
    #[must_use]
    pub fn default_domain(mut self) -> Self {
        self.is_default = true;
        self
    }

    /// Sets `favorite_orgs`.
    #[must_use]
    pub fn favorites(mut self, orgs: &[&str]) -> Self {
        self.favorite_orgs = orgs.iter().map(|org| (*org).to_owned()).collect();
        self
    }

    /// Sets `extra_orgs`.
    #[must_use]
    pub fn extras(mut self, orgs: &[&str]) -> Self {
        self.extra_orgs = orgs.iter().map(|org| (*org).to_owned()).collect();
        self
    }
}

/// Builder for `config.toml`; created by [`TestEnv::config`].
#[derive(Debug)]
pub struct ConfigSeed<'e> {
    env: &'e TestEnv,
    domains: Vec<DomainCfg>,
    include_orgs: Vec<String>,
    exclude_orgs: Vec<String>,
}

impl ConfigSeed<'_> {
    /// Adds a domain section.
    #[must_use]
    pub fn domain(mut self, domain: DomainCfg) -> Self {
        self.domains.push(domain);
        self
    }

    /// Sets `[filters] include_orgs`.
    #[must_use]
    pub fn include_orgs(mut self, orgs: &[&str]) -> Self {
        self.include_orgs = orgs.iter().map(|org| (*org).to_owned()).collect();
        self
    }

    /// Sets `[filters] exclude_orgs`.
    #[must_use]
    pub fn exclude_orgs(mut self, orgs: &[&str]) -> Self {
        self.exclude_orgs = orgs.iter().map(|org| (*org).to_owned()).collect();
        self
    }

    /// The TOML text that [`ConfigSeed::write`] saves.
    #[must_use]
    pub fn render(&self) -> String {
        let mut doc = DocumentMut::new();
        doc["work_dir"] = value(self.env.work_dir().display().to_string());
        if !self.include_orgs.is_empty() || !self.exclude_orgs.is_empty() {
            let mut filters = Table::new();
            if !self.include_orgs.is_empty() {
                filters["include_orgs"] = value(array(&self.include_orgs));
            }
            if !self.exclude_orgs.is_empty() {
                filters["exclude_orgs"] = value(array(&self.exclude_orgs));
            }
            doc["filters"] = Item::Table(filters);
        }
        if !self.domains.is_empty() {
            let mut domains = Table::new();
            domains.set_implicit(true);
            for domain in &self.domains {
                let mut table = Table::new();
                if domain.is_default {
                    table["is_default"] = value(true);
                }
                if !domain.favorite_orgs.is_empty() {
                    table["favorite_orgs"] = value(array(&domain.favorite_orgs));
                }
                if !domain.extra_orgs.is_empty() {
                    table["extra_orgs"] = value(array(&domain.extra_orgs));
                }
                domains.insert(&domain.host, Item::Table(table));
            }
            doc["domains"] = Item::Table(domains);
        }
        doc.to_string()
    }

    /// Writes `config.toml`, creating the config directory.
    pub fn write(&self) {
        let default = self
            .domains
            .iter()
            .find(|domain| domain.is_default)
            .map(|domain| domain.host.clone());
        *self
            .env
            .default_domain
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = default;
        write_file(&self.env.config_path(), &self.render());
    }
}

/// Builder for `cache.json` (and `frecency.json` when frecency is seeded);
/// created by [`TestEnv::cache`].
#[derive(Debug)]
pub struct CacheSeed<'e> {
    env: &'e TestEnv,
    repos: Vec<RepoSeed>,
    updated_at: Option<String>,
    frecency: Vec<(String, u32, String)>,
    query_frecency: Vec<(String, String, u32, String)>,
}

impl CacheSeed<'_> {
    /// Adds one repo.
    #[must_use]
    pub fn repo(mut self, repo: RepoSeed) -> Self {
        self.repos.push(repo);
        self
    }

    /// Adds several repos.
    #[must_use]
    pub fn repos(mut self, repos: &[RepoSeed]) -> Self {
        self.repos.extend_from_slice(repos);
        self
    }

    /// Sets the RFC 3339 `updated_at` (default: now).
    #[must_use]
    pub fn updated_at(mut self, updated_at: &str) -> Self {
        self.updated_at = Some(updated_at.to_owned());
        self
    }

    /// Makes the cache older than 24 hours.
    #[must_use]
    pub fn stale(self) -> Self {
        self.updated_at(STALE_TIME)
    }

    /// Seeds repo frecency: `slug` accessed `count` times, last at
    /// `last_access` (RFC 3339).
    #[must_use]
    pub fn frecency(mut self, slug: &str, count: u32, last_access: &str) -> Self {
        self.frecency
            .push((slug.to_owned(), count, last_access.to_owned()));
        self
    }

    /// Seeds query frecency: `query` resolved to `slug` `count` times, last
    /// at `last_access` (RFC 3339).
    #[must_use]
    pub fn query_frecency(
        mut self,
        query: &str,
        slug: &str,
        count: u32,
        last_access: &str,
    ) -> Self {
        self.query_frecency.push((
            query.to_owned(),
            slug.to_owned(),
            count,
            last_access.to_owned(),
        ));
        self
    }

    /// Writes `cache.json`, creating the state directory, and
    /// `frecency.json` when any frecency was seeded.
    pub fn write(&self) {
        let entry = |count: u32, last: &str| json!({ "access_count": count, "last_access": last });
        let repos: Vec<Value> = self
            .repos
            .iter()
            .map(|repo| {
                json!({
                    "slug": repo.slug,
                    "clone_url": repo.clone_url,
                    "html_url": repo.html_url,
                })
            })
            .collect();
        let mut frecency = Map::new();
        for (slug, count, last) in &self.frecency {
            frecency.insert(slug.clone(), entry(*count, last));
        }
        let mut query_frecency = Map::new();
        for (query, slug, count, last) in &self.query_frecency {
            query_frecency
                .entry(query.clone())
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
                .expect("object")
                .insert(slug.clone(), entry(*count, last));
        }
        let updated_at = self
            .updated_at
            .clone()
            .unwrap_or_else(|| jiff::Timestamp::now().to_string());
        let cache = json!({
            "version": 2,
            "updated_at": updated_at,
            "repos": repos,
        });
        write_file(&self.env.cache_path(), &cache.to_string());
        if !frecency.is_empty() || !query_frecency.is_empty() {
            let history = json!({
                "version": 1,
                "frecency": frecency,
                "query_frecency": query_frecency,
            });
            write_file(&self.env.frecency_path(), &history.to_string());
        }
    }
}

/// One cached repo for [`TestEnv::seed_cache`].
#[derive(Debug, Clone)]
pub struct RepoSeed {
    slug: String,
    clone_url: String,
    html_url: String,
}

impl RepoSeed {
    /// A repo for `slug` (`domain/org/repo`) with `https` clone and html
    /// URLs on the slug's domain.
    #[must_use]
    pub fn new(slug: &str) -> Self {
        Self {
            slug: slug.to_owned(),
            clone_url: format!("https://{slug}.git"),
            html_url: format!("https://{slug}"),
        }
    }

    /// Clones from `origin` (a `file://` URL) instead.
    #[must_use]
    pub fn origin(mut self, origin: &Origin) -> Self {
        self.clone_url = origin.url();
        self
    }
}

fn array(items: &[String]) -> Array {
    items.iter().map(String::as_str).collect()
}

fn write_file(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().expect("path has a parent")).expect("create parents");
    fs::write(path, text).expect("write file");
}

/// Resolves `program` on the host `PATH` to its canonical file.
fn find_on_path(program: &str) -> PathBuf {
    let path = std::env::var_os("PATH").expect("PATH is set");
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
        .and_then(|found| found.canonicalize().ok())
        .unwrap_or_else(|| panic!("`{program}` not found on PATH"))
}

/// Quotes `text` for a POSIX shell.
fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// Installs `git` into the shim dir as a wrapper that execs the host's real
/// git (canonicalised). A wrapper rather than a symlink so that a git which
/// is itself a script (a version-manager shim) still finds its
/// interpreter and tools: such a git keeps the host `PATH`.
fn install_git_wrapper(dest: &Path) {
    let git = find_on_path("git");
    let is_script = fs::read(&git).is_ok_and(|bytes| bytes.starts_with(b"#!"));
    let host_path = if is_script {
        let path = std::env::var("PATH").expect("PATH is valid UTF-8");
        format!("PATH={}; export PATH\n", sh_quote(&path))
    } else {
        String::new()
    };
    let script = format!(
        "#!/bin/sh\nif [ -n \"$WORK_SHIM_PROBE\" ]; then exit 0; fi\n{host_path}exec {} \"$@\"\n",
        sh_quote(&git.display().to_string())
    );
    install_script(dest, &script);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vars_are_hermetic_and_rooted_in_the_tempdir() {
        let env = TestEnv::new();
        let vars = env.vars();
        let get = |name: &str| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.clone())
        };
        assert_eq!(get("PATH"), Some(env.shim_dir().into()));
        assert_eq!(get("HOME"), Some(env.home().into()));
        assert_eq!(get("GITHUB_TOKEN"), Some("test-token".into()));
        assert_eq!(get("WORK_EDITOR"), Some(env.editor.path().into()));
        let mut keys: Vec<&str> = vars.iter().map(|(key, _)| *key).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "GITHUB_TOKEN",
                "GIT_AUTHOR_EMAIL",
                "GIT_AUTHOR_NAME",
                "GIT_COMMITTER_EMAIL",
                "GIT_COMMITTER_NAME",
                "GIT_CONFIG_GLOBAL",
                "GIT_CONFIG_NOSYSTEM",
                "HOME",
                "PATH",
                "WORK_EDITOR",
                "XDG_CONFIG_HOME",
                "XDG_DATA_HOME",
                "XDG_STATE_HOME",
            ]
        );
        for (key, value) in &vars {
            let value = value.to_string_lossy();
            let is_path = key.starts_with("XDG_")
                || matches!(*key, "HOME" | "PATH" | "WORK_EDITOR" | "GIT_CONFIG_GLOBAL");
            if is_path && value != "/dev/null" {
                assert!(
                    Path::new(&*value).starts_with(env.root()),
                    "{key}={value} leaks outside the tempdir"
                );
            }
        }
        let env = env.token(None);
        assert!(env.vars().iter().all(|(key, _)| *key != "GITHUB_TOKEN"));
    }

    #[test]
    fn command_runs_the_binary_without_the_host_environment() {
        let env = TestEnv::new();
        env.command().arg("--version").assert().success();
    }

    #[test]
    fn config_seed_writes_domains_and_filters() {
        let env = TestEnv::new();
        env.config()
            .domain(
                DomainCfg::new("github.com")
                    .default_domain()
                    .favorites(&["acme"]),
            )
            .domain(DomainCfg::new("ghe.example.com").extras(&["x"]))
            .exclude_orgs(&["old"])
            .include_orgs(&["a", "b"])
            .write();
        let text = fs::read_to_string(env.config_path()).expect("config written");
        let doc: toml_edit::DocumentMut = text.parse().expect("valid toml");
        assert_eq!(doc["work_dir"].as_str(), env.work_dir().to_str(), "{text}");
        let github = &doc["domains"]["github.com"];
        assert_eq!(github["is_default"].as_bool(), Some(true));
        assert_eq!(github["favorite_orgs"][0].as_str(), Some("acme"));
        assert_eq!(
            doc["domains"]["ghe.example.com"]["extra_orgs"][0].as_str(),
            Some("x")
        );
        assert_eq!(doc["filters"]["exclude_orgs"][0].as_str(), Some("old"));
        assert_eq!(doc["filters"]["include_orgs"][1].as_str(), Some("b"));

        env.write_config_raw("not = [valid");
        assert_eq!(
            fs::read_to_string(env.config_path()).expect("read"),
            "not = [valid"
        );
    }

    #[test]
    fn cache_seeding_writes_repos_and_staleness() {
        let env = TestEnv::new();
        let origin = env.origin("acme", "tool");
        env.seed_cache(&[
            RepoSeed::new("github.com/acme/app"),
            RepoSeed::new("github.com/acme/tool").origin(&origin),
        ]);
        let read = |env: &TestEnv| -> serde_json::Value {
            serde_json::from_str(&fs::read_to_string(env.cache_path()).expect("cache"))
                .expect("json")
        };
        let cache = read(&env);
        assert_eq!(cache["version"], 2);
        assert_eq!(
            cache["repos"][0]["clone_url"],
            "https://github.com/acme/app.git"
        );
        assert_eq!(cache["repos"][0]["html_url"], "https://github.com/acme/app");
        assert_eq!(cache["repos"][1]["clone_url"], origin.url());
        assert_ne!(cache["updated_at"], STALE_TIME);

        env.seed_stale_cache(&[]);
        assert_eq!(read(&env)["updated_at"], STALE_TIME);
        env.seed_cache_at(&[], "2030-01-01T00:00:00Z");
        assert_eq!(read(&env)["updated_at"], "2030-01-01T00:00:00Z");

        env.cache()
            .repo(RepoSeed::new("github.com/acme/app"))
            .stale()
            .frecency("github.com/acme/app", 4, "2026-01-02T03:04:05Z")
            .query_frecency("app", "github.com/acme/app", 3, "2026-01-02T03:04:05Z")
            .query_frecency("app", "github.com/acme/other", 1, "2026-01-01T00:00:00Z")
            .write();
        let cache = read(&env);
        assert_eq!(cache["updated_at"], STALE_TIME);
        assert_eq!(cache.get("frecency"), None);
        let cache: Value =
            serde_json::from_str(&fs::read_to_string(env.frecency_path()).expect("frecency"))
                .expect("json");
        assert_eq!(cache["version"], 1);
        assert_eq!(cache["frecency"]["github.com/acme/app"]["access_count"], 4);
        assert_eq!(
            cache["frecency"]["github.com/acme/app"]["last_access"],
            "2026-01-02T03:04:05Z"
        );
        assert_eq!(
            cache["query_frecency"]["app"]["github.com/acme/app"]["access_count"],
            3
        );
        assert_eq!(
            cache["query_frecency"]["app"]["github.com/acme/other"]["access_count"],
            1
        );
    }

    #[test]
    fn toml_is_escaped_by_toml_edit() {
        let env = TestEnv::new();
        env.config()
            .domain(DomainCfg::new("a.test").favorites(&["we\"ird\\\u{1F600}"]))
            .write();
        let text = fs::read_to_string(env.config_path()).expect("config");
        let doc: DocumentMut = text.parse().expect("valid toml");
        assert_eq!(
            doc["domains"]["a.test"]["favorite_orgs"][0].as_str(),
            Some("we\"ird\\\u{1F600}")
        );
    }

    #[test]
    fn repo_path_omits_the_default_domain() {
        let env = TestEnv::new();
        let work = env.work_dir();
        assert_eq!(
            env.repo_path("github.com/acme/app"),
            work.join("github.com/acme/app"),
            "no config yet, so no default domain"
        );
        env.config()
            .domain(DomainCfg::new("github.com").default_domain())
            .domain(DomainCfg::new("ghe.test"))
            .write();
        assert_eq!(env.repo_path("github.com/acme/app"), work.join("acme/app"));
        assert_eq!(
            env.repo_path("ghe.test/acme/app"),
            work.join("ghe.test/acme/app")
        );
    }

    #[test]
    fn path_has_sh_and_git_so_the_editor_can_be_launched() {
        let env = TestEnv::new();
        // Mirrors `sh -c "$WORK_EDITOR <path>"` with the controlled PATH.
        let status = std::process::Command::new(env.shim_dir().join("sh"))
            .arg("-c")
            .arg(format!("{} file.toml", env.editor.path().display()))
            .env_clear()
            .env("PATH", env.shim_dir())
            .status()
            .expect("run sh");
        assert!(status.success());
        assert_eq!(env.editor.calls(), [["file.toml"]]);
        let out = std::process::Command::new("git")
            .arg("--version")
            .env_clear()
            .env("PATH", env.shim_dir())
            .output()
            .expect("run git through PATH");
        assert!(String::from_utf8_lossy(&out.stdout).starts_with("git version"));
    }

    #[test]
    fn origins_clone_under_work_dir_and_shims_are_installed() {
        let env = TestEnv::new();
        let origin = env.origin("acme", "app");
        env.config()
            .domain(DomainCfg::new("github.com").default_domain())
            .write();
        let clone = env.clone_into_work_dir(&origin, "github.com/acme/app");
        assert_eq!(clone, env.work_dir().join("acme/app"));
        assert_eq!(git(&clone, &["rev-parse", "HEAD"]), origin.head());
        for shim in [&env.gh, &env.editor, &env.opener] {
            assert!(shim.path().is_file());
        }
        assert!(env.shim_dir().join("git").exists());
    }
}
