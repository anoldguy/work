//! `setup [domain]`.

use std::path::Path;
use std::process::ExitCode;

use crate::app::App;
use crate::auth::{AuthError, Tokens};
use crate::cache::refresh::{Freshness, load_or_refresh};
use crate::cli::SetupArgs;
use crate::config::locations::ExpandedPath;
use crate::config::{self, ConfigDoc};
use crate::domain::{Domain, Org};
use crate::error::Error;
use crate::gh::GhError;
use crate::github::ApiError;
use crate::text::plural;
use crate::ui::UiError;

/// The domain set up when none is named.
const DEFAULT_DOMAIN: &str = "github.com";

/// Prompts for `work_dir`, resolves tokens (offering `gh auth login` and
/// retrying on failure), adds the domain (as default if first), and offers an
/// initial update.
///
/// Running it again for a configured domain changes nothing but what the
/// prompts change. Existing comments in the config file are preserved.
///
/// # Errors
///
/// `UiError::NotInteractive` without a TTY, gh, token, or config write
/// failures.
pub fn run(app: &mut App, args: &SetupArgs) -> Result<ExitCode, Error> {
    if !app.ui.interact().is_interactive() {
        return Err(UiError::NotInteractive { what: "setup" }.into());
    }
    let fallback: Domain;
    let domain = if let Some(domain) = &args.domain {
        domain
    } else {
        fallback = DEFAULT_DOMAIN.parse()?;
        &fallback
    };

    let mut doc = ConfigDoc::open(&app.locations.config_file)?;
    let default_dir = display_dir(app.config.work_dir(), &app.locations.home);
    let typed = ask_work_dir(app, &default_dir)?;
    // An unchanged answer leaves an existing file's spelling alone.
    if typed != default_dir || !app.locations.config_file.exists() {
        doc.set_work_dir(&typed)?;
    }

    let has_token = ensure_token(app, domain)?;

    // The first domain becomes the default; a later one never changes it.
    let is_first = app.config.domains().is_empty();
    doc.add_domain(domain, is_first)?;
    doc.save()?;
    app.config = config::store::load(&app.locations)?;
    app.ui
        .stderr_line(&format!("Saved {}", app.locations.config_file.display()))?;

    if has_token && app.ui.interact().confirm("Fetch repositories now?", true)? {
        let cache = load_or_refresh(app, Freshness::Force { all_orgs: false })?;
        let repos = cache.repos.len();
        let noun = plural(repos, "repository", "repositories");
        app.ui.stderr_line(&format!("Cached {repos} {noun}"))?;
    }
    Ok(ExitCode::SUCCESS)
}

/// Prompts until the answer is a valid `work_dir` (the config loader's own
/// rules), so what `setup` writes always loads. Empty takes `default_dir`.
fn ask_work_dir(app: &mut App, default_dir: &str) -> Result<String, Error> {
    loop {
        let Some(typed) = app.ui.interact().input("Work directory", default_dir)? else {
            return Err(UiError::Interrupted.into());
        };
        let typed = typed.trim();
        let typed = if typed.is_empty() { default_dir } else { typed };
        match ExpandedPath::expand(typed, "work_dir", &app.locations.home) {
            Ok(_) => return Ok(typed.to_owned()),
            Err(err) => app.ui.stderr_line(&err.to_string())?,
        }
    }
}

/// What checking `domain`'s token found.
enum Probe {
    /// The API accepted it; the login it belongs to.
    Valid(Org),
    /// No token, or the API rejected it (401): `gh auth login` can help.
    NeedsLogin,
    /// The check itself failed (network, rate limit, ...).
    Unverified(ApiError),
}

/// Checks `domain`'s token, offering `gh auth login` when it is missing or
/// rejected and checking again afterwards. `Ok(false)` when the domain should
/// still be saved but has no verified token (declined, still failing).
fn ensure_token(app: &mut App, domain: &Domain) -> Result<bool, Error> {
    let mut probe = probe(app, domain)?;
    if matches!(probe, Probe::NeedsLogin) {
        let question = format!("No working token for {domain}. Run 'gh auth login' now?");
        if app.ui.interact().confirm(&question, true)? {
            // gh inherits stdio, so nothing of ours may still be buffered.
            app.ui.flush()?;
            app.gh.login(domain)?;
            probe = self::probe(app, domain)?;
        }
    }
    match probe {
        Probe::Valid(login) => {
            app.ui
                .stderr_line(&format!("Authenticated to {domain} as {login}"))?;
            Ok(true)
        }
        Probe::NeedsLogin => {
            app.ui.warn(&format!(
                "no token for {domain} (run 'gh auth login --hostname {domain}' later)"
            ));
            Ok(false)
        }
        Probe::Unverified(err) => {
            app.ui
                .warn(&format!("could not verify the token for {domain}: {err}"));
            Ok(false)
        }
    }
}

/// Resolves afresh (failures are never remembered) and tests the token with
/// `GET /user` under a spinner.
fn probe(app: &mut App, domain: &Domain) -> Result<Probe, Error> {
    let tokens = Tokens::resolve(
        &*app.gh,
        &app.env.github_token,
        std::slice::from_ref(domain),
    );
    let token = match tokens.get(domain) {
        Ok((token, _)) => token,
        Err(err @ (AuthError::InvalidEnvToken | AuthError::Gh(GhError::NotInstalled))) => {
            return Err(err.into());
        }
        Err(_) => return Ok(Probe::NeedsLogin),
    };
    let api = &*app.api;
    let result = app
        .ui
        .with_progress("Checking token", |_| api.viewer(domain, token));
    Ok(match result {
        Ok(login) => Probe::Valid(login),
        Err(ApiError::Status { status: 401, .. }) => Probe::NeedsLogin,
        Err(err) => Probe::Unverified(err),
    })
}

/// `dir` as the user would type it: `~/...` under `home`. A path that is not
/// UTF-8 has no faithful text form, so `~/code` is offered instead of lossy
/// text.
fn display_dir(dir: &Path, home: &Path) -> String {
    let shown = match dir.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => Some("~".to_owned()),
        Ok(rest) => rest.to_str().map(|r| format!("~/{r}")),
        Err(_) => dir.to_str().map(str::to_owned),
    };
    shown.unwrap_or_else(|| "~/code".to_owned())
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::fs;
    use std::sync::Arc;

    use super::*;
    use crate::app::Adapters;
    use crate::auth::EnvToken;
    use crate::auth::SecretToken;
    use crate::config::Locations;
    use crate::domain::{Org, Repo, StoredTime};
    use crate::env::EnvSnapshot;
    use crate::github::{ApiError, GitHubApi, Listing, Page};
    use crate::testing::Endpoint;
    use crate::testing::interact::PromptCall;
    use crate::testing::{
        Answer, FakeGh, FakeGit, FakeGitHub, PromptLog, ScriptedInteraction, SharedBuf,
    };
    use crate::ui::Ui;

    fn dom(s: &str) -> Domain {
        s.parse().expect("domain")
    }

    fn input(s: &str) -> Answer {
        Answer::Input(Some(s.to_owned()))
    }

    #[derive(Debug)]
    struct SharedApi(Arc<FakeGitHub>);

    impl GitHubApi for SharedApi {
        fn viewer(&self, d: &Domain, t: &SecretToken) -> Result<Org, ApiError> {
            self.0.viewer(d, t)
        }
        fn member_orgs(&self, d: &Domain, t: &SecretToken) -> Result<Vec<Org>, ApiError> {
            self.0.member_orgs(d, t)
        }
        fn user_repos(
            &self,
            d: &Domain,
            t: &SecretToken,
            on_page: &mut dyn FnMut(Page),
        ) -> Result<Listing, ApiError> {
            self.0.user_repos(d, t, on_page)
        }
        fn org_repos(
            &self,
            d: &Domain,
            t: &SecretToken,
            org: &Org,
            on_page: &mut dyn FnMut(Page),
        ) -> Result<Listing, ApiError> {
            self.0.org_repos(d, t, org, on_page)
        }
    }

    struct Harness {
        _dir: tempfile::TempDir,
        app: App,
        log: PromptLog,
        err: SharedBuf,
    }

    impl Harness {
        fn new(
            config: Option<&str>,
            interactive: bool,
            answers: Vec<Answer>,
            gh: FakeGh,
            api: Box<dyn GitHubApi>,
        ) -> Self {
            let dir = tempfile::tempdir().expect("tempdir");
            let env = EnvSnapshot {
                home: Some(dir.path().to_path_buf()),
                github_token: EnvToken::Unset,
                ..EnvSnapshot::default()
            };
            let locations = Locations::from_env(&env).expect("locations");
            if let Some(text) = config {
                fs::create_dir_all(locations.config_file.parent().expect("parent")).expect("mkdir");
                fs::write(&locations.config_file, text).expect("config");
            }
            let loaded = config::store::load(&locations).expect("config");
            let interaction = ScriptedInteraction::new(interactive, answers);
            let log = interaction.log();
            let err = SharedBuf::new();
            let ui = Ui::for_test(
                Box::new(SharedBuf::new()),
                Box::new(err.clone()),
                Box::new(interaction),
            );
            let adapters = Adapters {
                git: Box::new(FakeGit::new()),
                gh: Box::new(gh),
                api,
            };
            let now = StoredTime::from_unix_seconds(1_700_000_000).expect("time");
            let app = App::with_parts(env, locations, loaded, ui, adapters, now);
            Self {
                _dir: dir,
                app,
                log,
                err,
            }
        }

        fn run(&mut self, domain: Option<&str>) -> Result<ExitCode, Error> {
            run(
                &mut self.app,
                &SetupArgs {
                    domain: domain.map(dom),
                },
            )
        }

        fn config_text(&self) -> String {
            fs::read_to_string(&self.app.locations.config_file).expect("config file")
        }
    }

    fn viewer_api() -> Box<dyn GitHubApi> {
        Box::new(SharedApi(Arc::new(
            FakeGitHub::new()
                .with_viewer(dom("github.com"), "me".parse().expect("org"))
                .with_viewer(dom("ghe.example.com"), "me".parse().expect("org")),
        )))
    }

    fn with_token() -> FakeGh {
        FakeGh::new().with_token(dom("github.com"), "gho_abc")
    }

    #[test]
    fn without_a_terminal_it_is_a_one_line_error() {
        let mut h = Harness::new(None, false, vec![], FakeGh::new(), viewer_api());
        let err = h.run(None).expect_err("not interactive");
        assert_matches!(err, Error::Ui(UiError::NotInteractive { what: "setup" }));
        assert!(!err.to_string().contains('\n'));
        assert!(!h.app.locations.config_file.exists());
    }

    #[test]
    fn first_run_creates_the_config_with_the_default_domain() {
        let mut h = Harness::new(
            None,
            true,
            vec![input("~/src"), Answer::Confirm(false)],
            with_token(),
            viewer_api(),
        );
        h.run(None).expect("setup");
        let text = h.config_text();
        assert!(text.contains("work_dir = \"~/src\""), "{text}");
        assert!(text.contains("[domains.\"github.com\"]"), "{text}");
        assert!(text.contains("is_default = true"), "{text}");
        assert_matches!(
            &h.log.calls()[0],
            PromptCall::Input { default, .. } if default == "~/code"
        );
    }

    #[test]
    fn work_dir_prompt_defaults_to_the_current_value() {
        let mut h = Harness::new(
            Some("work_dir = \"~/projects\"\n"),
            true,
            vec![input("~/projects"), Answer::Confirm(false)],
            with_token(),
            viewer_api(),
        );
        h.run(None).expect("setup");
        assert_matches!(
            &h.log.calls()[0],
            PromptCall::Input { default, .. } if default == "~/projects"
        );
    }

    #[test]
    fn second_domain_is_added_without_changing_the_default() {
        let text = "# mine\n[domains.\"github.com\"]\nis_default = true\n";
        let gh = with_token().with_token(dom("ghe.example.com"), "gho_ghe");
        let mut h = Harness::new(
            Some(text),
            true,
            vec![input("~/code"), Answer::Confirm(false)],
            gh,
            viewer_api(),
        );
        h.run(Some("ghe.example.com")).expect("setup");
        let out = h.config_text();
        assert!(out.starts_with("# mine\n"), "{out}");
        assert!(out.contains("[domains.\"ghe.example.com\"]"), "{out}");
        assert_eq!(out.matches("is_default = true").count(), 1, "{out}");
        assert_eq!(
            h.app.config.default_domain().map(Domain::as_str),
            Some("github.com")
        );
        assert_eq!(h.app.config.domains().len(), 2);
    }

    #[test]
    fn rerunning_for_an_existing_domain_is_idempotent() {
        let text = "# keep me\nwork_dir = \"~/projects\"\n\n[domains.\"github.com\"]\nis_default = true\nfavorite_orgs = [\"acme\"]\n";
        let mut h = Harness::new(
            Some(text),
            true,
            vec![input("~/projects"), Answer::Confirm(false)],
            with_token(),
            viewer_api(),
        );
        h.run(None).expect("setup");
        assert_eq!(h.config_text(), text);
    }

    #[test]
    fn missing_token_offers_login_and_resolves_again() {
        let gh = FakeGh::new().with_token_after_login(dom("github.com"), "gho_new");
        let mut h = Harness::new(
            None,
            true,
            vec![
                input("~/code"),
                Answer::Confirm(true),
                Answer::Confirm(false),
            ],
            gh,
            viewer_api(),
        );
        h.run(None).expect("setup");
        assert!(h.log.entries().iter().any(|p| p.contains("gh auth login")));
        assert!(h.config_text().contains("github.com"));
    }

    #[test]
    fn login_that_yields_no_token_still_saves_with_a_warning() {
        let mut h = Harness::new(
            None,
            true,
            vec![input("~/src"), Answer::Confirm(true)],
            FakeGh::new(),
            viewer_api(),
        );
        h.run(None).expect("setup");
        let text = h.config_text();
        assert!(text.contains("work_dir = \"~/src\""), "{text}");
        assert!(text.contains("github.com"), "{text}");
        assert!(h.err.contents().contains("warning: no token"));
        assert_eq!(h.log.entries().len(), 2);
    }

    #[test]
    fn a_rejected_token_offers_login() {
        let api = FakeGitHub::new().with_status(dom("github.com"), Endpoint::Viewer, 401);
        let mut h = Harness::new(
            None,
            true,
            vec![input("~/code"), Answer::Confirm(false)],
            with_token(),
            Box::new(SharedApi(Arc::new(api))),
        );
        h.run(None).expect("setup");
        assert!(h.log.entries().iter().any(|p| p.contains("gh auth login")));
        assert!(h.err.contents().contains("warning: no token"));
    }

    #[test]
    fn a_failed_check_warns_and_skips_the_update_offer() {
        let api = FakeGitHub::new().with_status(dom("github.com"), Endpoint::Viewer, 500);
        let mut h = Harness::new(
            None,
            true,
            vec![input("~/code")],
            with_token(),
            Box::new(SharedApi(Arc::new(api))),
        );
        h.run(None).expect("setup");
        assert!(h.err.contents().contains("could not verify the token"));
        assert!(h.config_text().contains("github.com"));
    }

    #[test]
    fn a_working_token_is_reported() {
        let mut h = Harness::new(
            None,
            true,
            vec![input("~/code"), Answer::Confirm(false)],
            with_token(),
            viewer_api(),
        );
        h.run(None).expect("setup");
        assert!(
            h.err
                .contents()
                .contains("Authenticated to github.com as me"),
            "{}",
            h.err.contents()
        );
    }

    #[test]
    fn invalid_work_dir_answers_are_reprompted_and_never_saved() {
        let mut h = Harness::new(
            None,
            true,
            vec![
                input("relative/dir"),
                input("~other/code"),
                input("~/ok"),
                Answer::Confirm(false),
            ],
            with_token(),
            viewer_api(),
        );
        h.run(None).expect("setup");
        let text = h.config_text();
        assert!(text.contains("work_dir = \"~/ok\""), "{text}");
        assert!(!text.contains("relative"), "{text}");
        let stderr = h.err.contents();
        assert!(stderr.contains("must be absolute"), "{stderr}");
        assert!(stderr.contains("~user is not supported"), "{stderr}");
        assert!(config::store::load(&h.app.locations).is_ok());
    }

    #[test]
    fn an_empty_work_dir_answer_takes_the_default() {
        let mut h = Harness::new(
            None,
            true,
            vec![input("  "), Answer::Confirm(false)],
            with_token(),
            viewer_api(),
        );
        h.run(None).expect("setup");
        assert!(h.config_text().contains("work_dir = \"~/code\""));
    }

    #[test]
    fn declining_login_still_saves_but_skips_the_update_offer() {
        let mut h = Harness::new(
            None,
            true,
            vec![input("~/code"), Answer::Confirm(false)],
            FakeGh::new(),
            viewer_api(),
        );
        h.run(None).expect("setup");
        assert!(h.config_text().contains("github.com"));
        assert_eq!(h.log.entries().len(), 2);
        assert!(h.err.contents().contains("warning: no token"));
    }

    #[test]
    fn accepting_the_update_fetches_and_reports() {
        let repo: Repo = serde_json::from_value(serde_json::json!({
            "slug": "github.com/me/dotfiles",
            "clone_url": "https://github.com/me/dotfiles.git",
            "html_url": "https://github.com/me/dotfiles",
        }))
        .expect("repo");
        let api = FakeGitHub::new()
            .with_viewer(dom("github.com"), "me".parse().expect("org"))
            .with_user_repos(dom("github.com"), vec![vec![repo]]);
        let mut h = Harness::new(
            None,
            true,
            vec![input("~/code"), Answer::Confirm(true)],
            with_token(),
            Box::new(SharedApi(Arc::new(api))),
        );
        h.run(None).expect("setup");
        assert!(
            h.err.contents().contains("Cached 1 repository"),
            "{}",
            h.err.contents()
        );
    }

    #[test]
    fn cancelling_the_work_dir_prompt_is_an_interrupt() {
        let mut h = Harness::new(
            None,
            true,
            vec![Answer::Input(None)],
            with_token(),
            viewer_api(),
        );
        let err = h.run(None).expect_err("cancelled");
        assert_matches!(err, Error::Ui(UiError::Interrupted));
        assert!(!h.app.locations.config_file.exists());
    }

    #[test]
    fn display_dir_uses_tilde_under_home() {
        let home = Path::new("/home/u");
        assert_eq!(display_dir(Path::new("/home/u/code"), home), "~/code");
        assert_eq!(display_dir(Path::new("/home/u"), home), "~");
        assert_eq!(display_dir(Path::new("/srv/code"), home), "/srv/code");
    }
}
