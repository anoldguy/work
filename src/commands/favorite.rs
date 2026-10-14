//! `favorite [domain]`.

use std::process::ExitCode;

use crate::app::App;
use crate::auth::Tokens;
use crate::cli::FavoriteArgs;
use crate::config::{ConfigDoc, ConfigError};
use crate::domain::{Domain, Org};
use crate::error::Error;
use crate::ui::{Interaction, UiError};

/// Shown by the progress display while the orgs are fetched.
const PROGRESS_TITLE: &str = "Fetching organizations";

/// What the picker produced.
#[derive(Debug, PartialEq, Eq)]
enum Choice {
    /// Nothing to pick from.
    NoOrgs,
    /// The user cancelled; the config stays as it is.
    Cancelled,
    /// The new favourites (empty clears them).
    Selected(Vec<Org>),
}

/// Picks the domain (the only one, or a picker), fetches member and extra
/// orgs under a progress title, runs `pick_many` with current favourites
/// preselected, and saves them (empty clears).
///
/// # Errors
///
/// `UiError::NotInteractive` without a TTY, unknown domain, token, API, or
/// config write failures.
pub fn run(app: &mut App, args: FavoriteArgs) -> Result<ExitCode, Error> {
    if !app.ui.interact().is_interactive() {
        return Err(UiError::NotInteractive { what: "favorite" }.into());
    }
    let FavoriteArgs { domain } = args;
    let Some(domain) = select_domain(app, domain)? else {
        return Ok(ExitCode::SUCCESS);
    };
    let (extras, current) = {
        let cfg = app
            .config
            .domains()
            .get(&domain)
            .ok_or_else(|| ConfigError::UnknownDomain {
                domain: domain.clone(),
            })?;
        (cfg.extra_orgs.clone(), cfg.favorite_orgs.clone())
    };

    let domains = [domain.clone()];
    let tokens = Tokens::resolve(&*app.gh, &app.env.github_token, &domains);
    let (token, _) = tokens.get(&domain)?;
    let api = &*app.api;
    let mut orgs = app
        .ui
        .with_progress(PROGRESS_TITLE, |_| api.member_orgs(&domain, token))?;
    orgs.extend(extras);

    match choose_favorites(app.ui.interact(), &domain, &orgs, &current)? {
        Choice::NoOrgs => {
            app.ui.warn(&format!("no organizations found for {domain}"));
        }
        Choice::Cancelled => {
            app.ui.stderr_line("favorites unchanged")?;
        }
        Choice::Selected(favorites) => {
            let mut doc = ConfigDoc::open(&app.locations.config_file)?;
            doc.set_favorites(&domain, &favorites)?;
            doc.save()?;
            let line = if favorites.is_empty() {
                format!("Cleared favorite organizations for {domain}")
            } else {
                format!(
                    "Saved {} favorite organization{} for {domain}",
                    favorites.len(),
                    if favorites.len() == 1 { "" } else { "s" }
                )
            };
            app.ui.stdout_line(&line)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// The domain named, the only configured one, or the user's pick. `None`
/// when the picker was cancelled.
fn select_domain(app: &mut App, requested: Option<Domain>) -> Result<Option<Domain>, Error> {
    if let Some(domain) = requested {
        return if app.config.domains().contains_key(&domain) {
            Ok(Some(domain))
        } else {
            Err(ConfigError::UnknownDomain { domain }.into())
        };
    }
    let mut configured: Vec<Domain> = app.config.domains().keys().cloned().collect();
    match configured.len() {
        0 => {
            // `effective_domains` is never empty: it falls back to the implicit domain.
            let domain = app
                .config
                .effective_domains()
                .into_iter()
                .next()
                .expect("effective_domains is never empty");
            Err(ConfigError::UnknownDomain { domain }.into())
        }
        1 => Ok(configured.pop()),
        _ => {
            let names: Vec<&str> = configured.iter().map(Domain::as_str).collect();
            let picked = app.ui.interact().pick_one("Domain", &names)?;
            Ok(picked.and_then(|i| configured.into_iter().nth(i)))
        }
    }
}

/// Runs the multi-picker over the sorted union of `orgs` and `current`
/// (so a favourite the API no longer lists is not silently dropped), with
/// `current` preselected.
fn choose_favorites(
    interact: &mut dyn Interaction,
    domain: &Domain,
    orgs: &[Org],
    current: &[Org],
) -> Result<Choice, UiError> {
    let mut candidates: Vec<Org> = orgs.iter().chain(current).cloned().collect();
    candidates.sort();
    candidates.dedup();
    if candidates.is_empty() {
        return Ok(Choice::NoOrgs);
    }
    let names: Vec<&str> = candidates.iter().map(Org::as_str).collect();
    let selected: Vec<bool> = candidates.iter().map(|o| current.contains(o)).collect();
    let prompt = format!("Favorite organizations for {domain}");
    let Some(chosen) = interact.pick_many(&prompt, &names, &selected)? else {
        return Ok(Choice::Cancelled);
    };
    Ok(Choice::Selected(
        candidates
            .into_iter()
            .zip(chosen)
            .filter_map(|(org, keep)| keep.then_some(org))
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::app::Adapters;
    use crate::auth::{EnvToken, SecretToken};
    use crate::config::{self, Locations};
    use crate::domain::StoredTime;
    use crate::env::EnvSnapshot;
    use crate::testing::interact::PromptCall;
    use crate::testing::{Answer, FakeGh, FakeGit, FakeGitHub, ScriptedInteraction, SharedBuf};
    use crate::ui::Ui;

    const CONFIG: &str = "# my config\n[domains.\"github.com\"]\nis_default = true # keep\nfavorite_orgs = [\"acme\"]\nextra_orgs = [\"extra\"]\n";

    type RunOutcome = (Result<ExitCode, Error>, String, Vec<PromptCall>, String);

    /// Runs `favorite::run` for the only domain with the scripted `answers`;
    /// returns the outcome, the config text afterwards, the prompts shown
    /// and stdout.
    fn run_with(answers: Vec<Answer>) -> RunOutcome {
        let dir = tempfile::tempdir().expect("tempdir");
        let env = EnvSnapshot {
            home: Some(dir.path().to_path_buf()),
            github_token: EnvToken::Valid(SecretToken::new("ghp_test").expect("token")),
            ..EnvSnapshot::default()
        };
        let locations = Locations::from_env(&env).expect("locations");
        std::fs::create_dir_all(locations.config_file.parent().expect("parent")).expect("mkdir");
        std::fs::write(&locations.config_file, CONFIG).expect("config");
        let config = config::store::load(&locations).expect("config");
        let script = ScriptedInteraction::new(true, answers);
        let log = script.log();
        let out = SharedBuf::new();
        let ui = Ui::for_test(
            Box::new(out.clone()),
            Box::new(SharedBuf::new()),
            Box::new(script),
        );
        let api = FakeGitHub::new().with_member_orgs(dom(), orgs(&["acme", "widgets"]));
        let adapters = Adapters {
            git: Box::new(FakeGit::new()),
            gh: Box::new(FakeGh::new()),
            api: Box::new(api),
        };
        let now = StoredTime::from_unix_seconds(1_700_000_000).expect("time");
        let mut app = App::with_parts(env, locations, config, ui, adapters, now);
        let result = run(&mut app, FavoriteArgs { domain: None });
        app.ui.flush().expect("flush");
        let text = std::fs::read_to_string(&app.locations.config_file).expect("config");
        (result, text, log.calls(), out.contents())
    }

    #[test]
    fn run_saves_the_selection_and_keeps_comments() {
        // Items: acme, extra, widgets; acme is preselected.
        let (result, text, calls, out) =
            run_with(vec![Answer::PickMany(Some(vec![false, true, true]))]);
        assert_matches!(result, Ok(_));
        assert!(text.starts_with("# my config\n"), "{text}");
        assert!(text.contains("# keep"), "{text}");
        assert!(
            text.contains("favorite_orgs = [\"extra\", \"widgets\"]"),
            "{text}"
        );
        assert_matches!(
            calls.as_slice(),
            [PromptCall::PickMany { items, selected, .. }]
                if items == &["acme", "extra", "widgets"] && selected == &[true, false, false]
        );
        assert_eq!(out, "Saved 2 favorite organizations for github.com\n");
    }

    #[test]
    fn run_clears_favorites_on_an_empty_selection() {
        let (result, text, _, out) =
            run_with(vec![Answer::PickMany(Some(vec![false, false, false]))]);
        assert_matches!(result, Ok(_));
        assert!(!text.contains("favorite_orgs"), "{text}");
        assert!(text.contains("extra_orgs"), "{text}");
        assert_eq!(out, "Cleared favorite organizations for github.com\n");
    }

    #[test]
    fn run_leaves_the_config_alone_on_cancel() {
        let (result, text, _, out) = run_with(vec![Answer::PickMany(None)]);
        assert_matches!(result, Ok(_));
        assert_eq!(text, CONFIG);
        assert_eq!(out, "");
    }

    fn org(s: &str) -> Org {
        s.parse().expect("org")
    }

    fn orgs(names: &[&str]) -> Vec<Org> {
        names.iter().map(|n| org(n)).collect()
    }

    fn dom() -> Domain {
        "github.com".parse().expect("domain")
    }

    #[test]
    fn current_favorites_are_preselected_and_chosen_orgs_returned() {
        let mut script =
            ScriptedInteraction::new(true, vec![Answer::PickMany(Some(vec![true, false, true]))]);
        let log = script.log();
        let choice = choose_favorites(
            &mut script,
            &dom(),
            &orgs(&["widgets", "acme"]),
            &orgs(&["acme", "zeta"]),
        )
        .expect("choice");
        assert_eq!(choice, Choice::Selected(orgs(&["acme", "zeta"])));
        assert_matches!(
            log.calls().as_slice(),
            [PromptCall::PickMany { items, selected, .. }]
                if items == &["acme", "widgets", "zeta"]
                    && selected == &[true, false, true]
        );
    }

    #[test]
    fn deselecting_everything_clears() {
        let mut script =
            ScriptedInteraction::new(true, vec![Answer::PickMany(Some(vec![false, false]))]);
        let choice = choose_favorites(&mut script, &dom(), &orgs(&["a", "b"]), &orgs(&["a"]))
            .expect("choice");
        assert_eq!(choice, Choice::Selected(Vec::new()));
    }

    #[test]
    fn cancelling_changes_nothing() {
        let mut script = ScriptedInteraction::new(true, vec![Answer::PickMany(None)]);
        let choice = choose_favorites(&mut script, &dom(), &orgs(&["a"]), &[]).expect("choice");
        assert_eq!(choice, Choice::Cancelled);
    }

    #[test]
    fn no_orgs_skips_the_picker() {
        let mut script = ScriptedInteraction::new(true, vec![]);
        let choice = choose_favorites(&mut script, &dom(), &[], &[]).expect("choice");
        assert_eq!(choice, Choice::NoOrgs);
        assert_eq!(script.log().calls().len(), 0);
    }

    #[test]
    fn interrupt_propagates() {
        let mut script = ScriptedInteraction::new(true, vec![Answer::Interrupt]);
        let result = choose_favorites(&mut script, &dom(), &orgs(&["a"]), &[]);
        assert_matches!(result, Err(UiError::Interrupted));
    }
}
