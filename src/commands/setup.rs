//! `setup [domain]`.

use std::path::Path;
use std::process::ExitCode;

use crate::app::App;
use crate::cache::refresh::{Freshness, load_or_refresh};
use crate::cli::SetupArgs;
use crate::config::locations::ExpandedPath;
use crate::config::{self, ConfigDoc};
use crate::domain::{Domain, Org};
use crate::error::Error;
use crate::gh::GhError;
use crate::proc::Interactive;
use crate::text::plural;
use crate::ui::Ui;
use crate::ui::UiError;

/// The domain set up when none is named.
const DEFAULT_DOMAIN: &str = "github.com";

/// Prompts for `work_dir`, checks gh's login (offering `gh auth login` and
/// checking again on failure), adds the domain (as default if first), and offers an
/// initial update.
///
/// Running it again for a configured domain changes nothing but what the
/// prompts change. Existing comments in the config file are preserved.
///
/// # Errors
///
/// `UiError::NotInteractive` without a TTY, a missing gh, or config write
/// failures.
pub fn run(app: App, ui: &mut Ui, args: &SetupArgs) -> Result<ExitCode, Error> {
    if !ui.interact().is_interactive() {
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
    let typed = ask_work_dir(&app, ui, &default_dir)?;
    // An unchanged answer leaves an existing file's spelling alone.
    if typed != default_dir || !app.locations.config_file.exists() {
        doc.set_work_dir(&typed)?;
    }

    let logged_in = ensure_login(&app, ui, domain)?;

    // The first domain becomes the default; a later one never changes it.
    let is_first = app.config.domains().is_empty();
    doc.add_domain(domain, is_first)?;
    doc.save()?;
    let app = App {
        config: config::store::load(&app.locations)?,
        ..app
    };
    ui.stderr_line(&format!("Saved {}", app.locations.config_file.display()))?;

    if logged_in && ui.interact().confirm("Fetch repositories now?", true)? {
        let cache = load_or_refresh(&app, ui, Freshness::Force { all_orgs: false })?;
        let repos = cache.repos.len();
        let noun = plural(repos, "repository", "repositories");
        ui.stderr_line(&format!("Cached {repos} {noun}"))?;
    }
    Ok(ExitCode::SUCCESS)
}

/// Prompts until the answer is a valid `work_dir` (the config loader's own
/// rules), so what `setup` writes always loads. Empty takes `default_dir`.
fn ask_work_dir(app: &App, ui: &mut Ui, default_dir: &str) -> Result<String, Error> {
    loop {
        let Some(typed) = ui.interact().input("Work directory", default_dir)? else {
            return Err(UiError::Interrupted.into());
        };
        let typed = typed.trim();
        let typed = if typed.is_empty() { default_dir } else { typed };
        match ExpandedPath::expand(typed, "work_dir", &app.locations.home) {
            Ok(_) => return Ok(typed.to_owned()),
            Err(err) => ui.stderr_line(&err.to_string())?,
        }
    }
}

/// Checks that gh is logged in to `domain` (`gh api user`), offering
/// `gh auth login` when the check fails and checking again afterwards.
/// `Ok(false)` when the domain should still be saved but gh could not reach
/// it (declined, still failing).
fn ensure_login(app: &App, ui: &mut Ui, domain: &Domain) -> Result<bool, Error> {
    let mut check = check_login(app, ui, domain)?;
    if let Err(err) = &check {
        let question = format!("gh could not reach {domain} ({err}). Run 'gh auth login' now?");
        if ui.interact().confirm(&question, true)? {
            app.gh.login(&mut Interactive::new(ui)?, domain)?;
            check = check_login(app, ui, domain)?;
        }
    }
    match check {
        Ok(login) => {
            ui.stderr_line(&format!("Authenticated to {domain} as {login}"))?;
            Ok(true)
        }
        Err(err) => {
            ui.warn(&format!("gh is not logged in to {domain}: {err}"));
            Ok(false)
        }
    }
}

/// Asks gh for the viewer under a spinner. The inner `Err` is gh's own
/// one-line reason; a missing or unstartable `gh` is the outer error.
fn check_login(app: &App, ui: &mut Ui, domain: &Domain) -> Result<Result<Org, GhError>, Error> {
    let gh = &*app.gh;
    let result = ui.with_progress("Checking gh login", |_| gh.viewer(domain));
    match result {
        Err(err @ (GhError::NotInstalled | GhError::Spawn { .. })) => Err(err.into()),
        other => Ok(other),
    }
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

    use super::*;
    use crate::app::Adapters;
    use crate::config::Locations;
    use crate::domain::{Repo, StoredTime};
    use crate::env::EnvSnapshot;
    use crate::testing::interact::PromptCall;
    use crate::testing::{Answer, FakeGh, FakeGit, PromptLog, ScriptedInteraction, SharedBuf};
    use crate::ui::Ui;

    fn dom(s: &str) -> Domain {
        s.parse().expect("domain")
    }

    fn me() -> Org {
        "me".parse().expect("org")
    }

    fn input(s: &str) -> Answer {
        Answer::Input(Some(s.to_owned()))
    }

    struct Harness {
        _dir: tempfile::TempDir,
        app: Option<App>,
        locations: Locations,
        ui: Ui,
        log: PromptLog,
        err: SharedBuf,
    }

    impl Harness {
        fn new(config: Option<&str>, interactive: bool, answers: Vec<Answer>, gh: FakeGh) -> Self {
            let dir = tempfile::tempdir().expect("tempdir");
            let env = EnvSnapshot {
                home: Some(dir.path().to_path_buf()),
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
            };
            let now = StoredTime::from_unix_seconds(1_700_000_000).expect("time");
            let app = App::with_parts(env, locations.clone(), loaded, adapters, now);
            Self {
                _dir: dir,
                app: Some(app),
                locations,
                ui,
                log,
                err,
            }
        }

        fn run(&mut self, domain: Option<&str>) -> Result<ExitCode, Error> {
            run(
                self.app.take().expect("one run per harness"),
                &mut self.ui,
                &SetupArgs {
                    domain: domain.map(dom),
                },
            )
        }

        fn config_text(&self) -> String {
            fs::read_to_string(&self.locations.config_file).expect("config file")
        }
    }

    fn logged_in() -> FakeGh {
        FakeGh::new()
            .with_viewer(dom("github.com"), me())
            .with_viewer(dom("ghe.example.com"), me())
    }

    #[test]
    fn without_a_terminal_it_is_a_one_line_error() {
        let mut h = Harness::new(None, false, vec![], FakeGh::new());
        let err = h.run(None).expect_err("not interactive");
        assert_matches!(err, Error::Ui(UiError::NotInteractive { what: "setup" }));
        assert!(!err.to_string().contains('\n'));
        assert!(!h.locations.config_file.exists());
    }

    #[test]
    fn first_run_creates_the_config_with_the_default_domain() {
        let mut h = Harness::new(
            None,
            true,
            vec![input("~/src"), Answer::Confirm(false)],
            logged_in(),
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
        assert!(
            h.err
                .contents()
                .contains("Authenticated to github.com as me"),
            "{}",
            h.err.contents()
        );
    }

    #[test]
    fn second_domain_is_added_without_changing_the_default() {
        let text = "# mine\n[domains.\"github.com\"]\nis_default = true\n";
        let mut h = Harness::new(
            Some(text),
            true,
            vec![input("~/code"), Answer::Confirm(false)],
            logged_in(),
        );
        h.run(Some("ghe.example.com")).expect("setup");
        let out = h.config_text();
        assert!(out.starts_with("# mine\n"), "{out}");
        assert!(out.contains("[domains.\"ghe.example.com\"]"), "{out}");
        assert_eq!(out.matches("is_default = true").count(), 1, "{out}");
        assert_eq!(
            config::store::load(&h.locations)
                .expect("config")
                .default_domain()
                .map(Domain::as_str),
            Some("github.com")
        );
    }

    #[test]
    fn rerunning_for_an_existing_domain_is_idempotent() {
        let text = "# keep me\nwork_dir = \"~/projects\"\n\n[domains.\"github.com\"]\nis_default = true\nfavorite_orgs = [\"acme\"]\n";
        let mut h = Harness::new(
            Some(text),
            true,
            vec![input("~/projects"), Answer::Confirm(false)],
            logged_in(),
        );
        h.run(None).expect("setup");
        assert_eq!(h.config_text(), text);
    }

    #[test]
    fn not_logged_in_offers_login_and_checks_again() {
        let gh = FakeGh::new().with_viewer_after_login(dom("github.com"), me());
        let mut h = Harness::new(
            None,
            true,
            vec![
                input("~/code"),
                Answer::Confirm(true),
                Answer::Confirm(false),
            ],
            gh,
        );
        h.run(None).expect("setup");
        assert!(h.log.entries().iter().any(|p| p.contains("gh auth login")));
        assert!(
            h.err
                .contents()
                .contains("Authenticated to github.com as me")
        );
        assert!(h.config_text().contains("github.com"));
    }

    #[test]
    fn declining_login_still_saves_but_skips_the_update_offer() {
        let mut h = Harness::new(
            None,
            true,
            vec![input("~/code"), Answer::Confirm(false)],
            FakeGh::new(),
        );
        h.run(None).expect("setup");
        assert!(h.config_text().contains("github.com"));
        // The work_dir and login prompts only: no update offer.
        assert_eq!(h.log.entries().len(), 2);
        let err = h.err.contents();
        assert!(
            err.contains("warning: gh is not logged in to github.com"),
            "{err}"
        );
        assert!(!err.contains('\u{1b}'), "{err:?}");
    }

    #[test]
    fn accepting_the_update_fetches_and_reports() {
        let repo: Repo = serde_json::from_value(serde_json::json!({
            "slug": "github.com/me/dotfiles",
            "clone_url": "https://github.com/me/dotfiles.git",
            "html_url": "https://github.com/me/dotfiles",
        }))
        .expect("repo");
        let gh = logged_in().with_user_repos(dom("github.com"), vec![repo]);
        let mut h = Harness::new(None, true, vec![input("~/code"), Answer::Confirm(true)], gh);
        h.run(None).expect("setup");
        assert!(
            h.err.contents().contains("Cached 1 repository"),
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
            logged_in(),
        );
        h.run(None).expect("setup");
        let text = h.config_text();
        assert!(text.contains("work_dir = \"~/ok\""), "{text}");
        assert!(!text.contains("relative"), "{text}");
        assert!(config::store::load(&h.locations).is_ok());
    }

    #[test]
    fn cancelling_the_work_dir_prompt_is_an_interrupt() {
        let mut h = Harness::new(None, true, vec![Answer::Input(None)], logged_in());
        let err = h.run(None).expect_err("cancelled");
        assert_matches!(err, Error::Ui(UiError::Interrupted));
        assert!(!h.locations.config_file.exists());
    }
}
