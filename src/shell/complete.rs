//! The completion engine behind the hidden `sh-complete`: given the words
//! typed so far, what can go at the cursor.
//!
//! It walks the clap command tree itself, so a new subcommand or flag
//! completes without touching any shell script.

use clap::{Arg, Command};

use crate::domain::{Domain, Repo, Slug};
use crate::text::one_line;

/// One thing the shell can offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The word to insert.
    pub value: String,
    /// A one-line description, for shells that show one.
    pub help: Option<String>,
}

impl Candidate {
    fn new(value: impl Into<String>, help: Option<String>) -> Self {
        Self {
            value: value.into(),
            help,
        }
    }
}

/// Values that come from the user's state rather than the command tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Every known repo: cached, plus adopted on a configured domain.
    Repos,
    /// Adopted repos only, on any domain.
    AdoptedRepos,
    /// The configured domains.
    Domains,
}

/// What can go at the cursor.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Slot {
    /// Subcommands, flags and fixed values, already matched against the
    /// current word.
    pub fixed: Vec<Candidate>,
    /// The state to draw further values from, if the slot takes any.
    pub source: Option<Source>,
    /// The part of the current word that such values must start with.
    pub partial: String,
    /// The part of the current word before `partial` (`--flag=`), which
    /// every such value must keep.
    pub prefix: String,
}

/// The slot at the last of `words` (the words after the program name; the
/// last one is the word under the cursor, possibly empty). `cmd` must be
/// built ([`Command::build`]), so that `help` and `--help` exist.
#[must_use]
pub fn slot(cmd: &Command, words: &[String]) -> Slot {
    let Some((current, before)) = words.split_last() else {
        return Slot::default();
    };
    let mut at = Cursor {
        cmd,
        positionals: 0,
        seen: Vec::new(),
        awaiting: None,
        literal: false,
    };
    for word in before {
        at.advance(word);
    }
    at.slot(current)
}

/// Where the walk over the words before the cursor has got to.
struct Cursor<'c> {
    cmd: &'c Command,
    /// Positional arguments of `cmd` already given.
    positionals: usize,
    /// Flags of `cmd` already given.
    seen: Vec<&'c Arg>,
    /// The option whose value the next word is.
    awaiting: Option<&'c Arg>,
    /// After `--`: every word is a positional.
    literal: bool,
}

impl<'c> Cursor<'c> {
    fn advance(&mut self, word: &str) {
        if self.awaiting.take().is_some() {
            return;
        }
        if self.literal {
            self.positionals += 1;
        } else if word == "--" {
            self.literal = true;
        } else if let Some(long) = word.strip_prefix("--") {
            let (name, value) = match long.split_once('=') {
                Some((name, _)) => (name, true),
                None => (long, false),
            };
            if let Some(arg) = self.long(name) {
                self.seen.push(arg);
                if !value && takes_value(arg) {
                    self.awaiting = Some(arg);
                }
            }
        } else if let Some(shorts) = word.strip_prefix('-').filter(|s| !s.is_empty()) {
            // `-j8` carries its value; `-j` waits for one.
            let mut chars = shorts.chars();
            while let Some(arg) = chars.next().and_then(|c| self.short(c)) {
                self.seen.push(arg);
                if takes_value(arg) {
                    if chars.next().is_none() {
                        self.awaiting = Some(arg);
                    }
                    break;
                }
            }
        } else if let Some(sub) = self.cmd.find_subcommand(word) {
            self.cmd = sub;
            self.positionals = 0;
            self.seen.clear();
        } else {
            self.positionals += 1;
        }
    }

    fn slot(&self, current: &str) -> Slot {
        if let Some(arg) = self.awaiting {
            return self.values(arg, "", current);
        }
        if !self.literal && current.starts_with('-') {
            if let Some((flag, partial)) = current.split_once('=') {
                return flag
                    .strip_prefix("--")
                    .and_then(|name| self.long(name))
                    .filter(|arg| takes_value(arg))
                    .map(|arg| self.values(arg, &format!("{flag}="), partial))
                    .unwrap_or_default();
            }
            return Slot {
                fixed: self.flags(current),
                ..Slot::default()
            };
        }
        let mut slot = self
            .cmd
            .get_positionals()
            .find(|arg| arg.get_index() == Some(self.positionals + 1))
            .map(|arg| self.values(arg, "", current))
            .unwrap_or_default();
        if !self.literal && self.positionals == 0 {
            let subcommands = self
                .cmd
                .get_subcommands()
                .filter(|sub| !sub.is_hide_set() && sub.get_name().starts_with(current))
                .map(|sub| Candidate::new(sub.get_name(), sub.get_about().map(help_line)));
            slot.fixed.splice(0..0, subcommands);
        }
        slot
    }

    /// The visible flags not given yet that start with `current`.
    fn flags(&self, current: &str) -> Vec<Candidate> {
        self.cmd
            .get_arguments()
            .filter(|arg| !arg.is_positional() && !arg.is_hide_set())
            .filter(|arg| !self.seen.iter().any(|seen| seen.get_id() == arg.get_id()))
            .filter_map(|arg| {
                let flag = match (arg.get_long(), arg.get_short()) {
                    (Some(long), _) => format!("--{long}"),
                    (None, Some(short)) => format!("-{short}"),
                    (None, None) => return None,
                };
                flag.starts_with(current)
                    .then(|| Candidate::new(flag, arg.get_help().map(help_line)))
            })
            .collect()
    }

    /// The values `arg` takes: its fixed set if it has one, else its source.
    fn values(&self, arg: &Arg, prefix: &str, partial: &str) -> Slot {
        let fixed: Vec<Candidate> = arg
            .get_possible_values()
            .iter()
            .filter(|v| !v.is_hide_set() && v.get_name().starts_with(partial))
            .map(|v| Candidate::new(format!("{prefix}{}", v.get_name()), None))
            .collect();
        let source = if arg.get_possible_values().is_empty() {
            source(self.cmd, arg)
        } else {
            None
        };
        Slot {
            fixed,
            source,
            partial: partial.to_owned(),
            prefix: prefix.to_owned(),
        }
    }

    fn long(&self, name: &str) -> Option<&'c Arg> {
        self.cmd
            .get_arguments()
            .find(|arg| arg.get_long() == Some(name))
    }

    fn short(&self, name: char) -> Option<&'c Arg> {
        self.cmd
            .get_arguments()
            .find(|arg| arg.get_short() == Some(name))
    }
}

fn takes_value(arg: &Arg) -> bool {
    arg.get_action().takes_values()
}

fn help_line(help: &clap::builder::StyledStr) -> String {
    one_line(&help.to_string())
}

/// Where the values of `arg` (an argument of `cmd`) come from. Keyed on the
/// argument's field name in `cli`: every `query` is a repo, every `domain` a
/// domain.
fn source(cmd: &Command, arg: &Arg) -> Option<Source> {
    match arg.get_id().as_str() {
        "query" if cmd.get_name() == "forget" => Some(Source::AdoptedRepos),
        "query" => Some(Source::Repos),
        "domain" => Some(Source::Domains),
        _ => None,
    }
}

fn starts_with_ignore_case(text: &str, prefix: &str) -> bool {
    text.as_bytes()
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix.as_bytes()))
}

/// The ways of naming `repos` that start with `partial` (ASCII case
/// ignored), highest `score` first, then alphabetically.
///
/// Each slash typed asks for a longer form, as in a query: no slash offers
/// `name` (and `org/name` once the org is being typed), one offers
/// `org/name` and `domain/org/name`, two offer the full slug.
#[must_use]
pub fn repo_values(repos: &[Repo], partial: &str, score: impl Fn(&Slug) -> f64) -> Vec<String> {
    let slashes = partial.matches('/').count();
    let mut best: Vec<(String, f64)> = Vec::new();
    for repo in repos {
        let slug = &repo.slug;
        let (domain, org, name) = (slug.domain(), slug.org(), slug.name());
        let forms = [
            (slashes == 0).then(|| name.to_string()),
            (slashes == 1 || (slashes == 0 && !partial.is_empty()))
                .then(|| format!("{org}/{name}")),
            (slashes >= 1).then(|| format!("{domain}/{org}/{name}")),
        ];
        let points = score(slug);
        for form in forms.into_iter().flatten() {
            if starts_with_ignore_case(&form, partial) {
                best.push((form, points));
            }
        }
    }
    // Several repos can share a name: keep each form once, at its best score.
    best.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.total_cmp(&a.1)));
    best.dedup_by(|later, first| later.0 == first.0);
    best.sort_by(|a, b| b.1.total_cmp(&a.1));
    best.into_iter().map(|(form, _)| form).collect()
}

/// The `domains` that start with `partial`, ASCII case ignored.
#[must_use]
pub fn domain_values(domains: &[Domain], partial: &str) -> Vec<String> {
    domains
        .iter()
        .map(Domain::as_str)
        .filter(|domain| starts_with_ignore_case(domain, partial))
        .map(str::to_owned)
        .collect()
}
