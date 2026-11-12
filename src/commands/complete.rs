//! The hidden `sh-complete <shell> -- <words>`: completion candidates for
//! the stubs `completions` prints.

use std::process::ExitCode;

use clap::CommandFactory;

use crate::adopted;
use crate::cache::store::{self, LoadOutcome};
use crate::cli::{Cli, ShCompleteArgs};
use crate::config::{Config, Locations, store as config_store};
use crate::domain::{Repo, StoredTime};
use crate::env::EnvSnapshot;
use crate::frecency;
use crate::shell::Shell;
use crate::shell::complete::{Candidate, Slot, Source, domain_values, repo_values, slot};
use crate::ui::Ui;

/// Prints one candidate per line for the word under the cursor, in the
/// form `shell`'s stub reads, from the command tree and the files on disk:
/// no network, and no refresh of a stale cache. Prints only what it could
/// work out and exits 0 on any failure, because a shell runs it on every
/// Tab press.
#[must_use]
pub fn run(env: &EnvSnapshot, ui: &mut Ui, args: &ShCompleteArgs) -> ExitCode {
    let mut cmd = Cli::command();
    cmd.build();
    let words = match args.shell {
        Shell::Bash => rejoin_bash(&args.words),
        Shell::Zsh | Shell::Fish => args.words.clone(),
    };
    let Slot {
        mut fixed,
        source,
        partial,
        prefix,
    } = slot(&cmd, &words);
    if let Some(source) = source {
        let values = stored_values(env, source, &partial).unwrap_or_default();
        fixed.extend(values.into_iter().map(|value| Candidate {
            value: format!("{prefix}{value}"),
            help: None,
        }));
    }
    for candidate in &fixed {
        if ui.stdout_line(&render(args.shell, candidate)).is_err() {
            break;
        }
    }
    ExitCode::SUCCESS
}

/// Bash splits words at `=`, so `--domain=gh` arrives as `--domain`, `=`,
/// `gh`: dropping the `=` leaves the equivalent `--domain gh`, whose last
/// word is the one bash will replace.
fn rejoin_bash(words: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    for (i, word) in words.iter().enumerate() {
        let after_flag = out.last().is_some_and(|prev| prev.starts_with("--"));
        if word == "=" && after_flag {
            // `--domain=<Tab>`: the `=` is the word under the cursor.
            if i + 1 == words.len() {
                out.push(String::new());
            }
        } else {
            out.push(word.clone());
        }
    }
    out
}

fn stored_values(env: &EnvSnapshot, source: Source, partial: &str) -> Option<Vec<String>> {
    let locations = Locations::from_env(env).ok()?;
    let config = config_store::load(&locations).ok()?;
    let repos = match source {
        Source::Domains => return Some(domain_values(&config.effective_domains(), partial)),
        Source::Repos => known_repos(&config, false)?,
        Source::AdoptedRepos => known_repos(&config, true)?,
    };
    let (history, _) = frecency::load(config.cache_dir());
    let now = StoredTime::now();
    Some(repo_values(&repos, partial, |slug| {
        history.frecency.score(slug, now)
    }))
}

/// The repos known without any network: the cache plus the adopted repos
/// on configured domains, or with `adopted_only` the adopted repos on any
/// domain. `None` without a readable cache.
#[must_use]
pub fn known_repos(config: &Config, adopted_only: bool) -> Option<Vec<Repo>> {
    let (adopted, _) = adopted::load(config.cache_dir());
    if adopted_only {
        return Some(adopted.repos);
    }
    let LoadOutcome::Loaded(mut cache) = store::load(config.cache_dir()) else {
        return None;
    };
    adopted::extend_known(&mut cache.repos, adopted, &config.effective_domains());
    Some(cache.repos)
}

/// One output line: the bare value for bash, `value<Tab>help` for fish,
/// and `value:help` (colons in the value escaped) for zsh's `_describe`.
fn render(shell: Shell, Candidate { value, help }: &Candidate) -> String {
    let help = help.as_deref().filter(|help| !help.is_empty());
    match (shell, help) {
        (Shell::Bash, _) | (Shell::Fish, None) => value.clone(),
        (Shell::Fish, Some(help)) => format!("{value}\t{help}"),
        (Shell::Zsh, None) => value.replace(':', "\\:"),
        (Shell::Zsh, Some(help)) => format!("{}:{help}", value.replace(':', "\\:")),
    }
}
