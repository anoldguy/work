//! Progress for long-running work: workers report through [`Tasks`] and
//! [`Task`], which drive `indicatif` bars.

use std::mem;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

use crate::text::{one_line, plural};

/// What a progress display counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Noun {
    /// Repo listings.
    Sources,
    /// Repositories.
    Repositories,
}

impl Noun {
    /// The word for `n` items: `source`/`sources`,
    /// `repository`/`repositories`.
    #[must_use]
    pub fn word(self, n: u32) -> &'static str {
        match self {
            Self::Sources => plural(n, "source", "sources"),
            Self::Repositories => plural(n, "repository", "repositories"),
        }
    }
}

/// The action a live display describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    /// Updating repo listings.
    Updating,
    /// Syncing repositories.
    Syncing,
}

impl Verb {
    /// `Updating` or `Syncing`.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Updating => "Updating",
            Self::Syncing => "Syncing",
        }
    }
}

/// A fixed in-flight step of a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// `git fetch`.
    Fetching,
    /// `git merge --ff-only`.
    FastForwarding,
}

impl Step {
    /// The in-flight text: `fetching…` or `fast-forwarding`.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Fetching => "fetching…",
            Self::FastForwarding => "fast-forwarding",
        }
    }
}

/// The first line of a live display, e.g.
/// `Updating 12 sources across 2 domains  [7/12]` or
/// `Syncing 40 repositories  [12/40]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// What is happening.
    pub verb: Verb,
    /// Number of tasks that will be started.
    pub total: u32,
    /// What the tasks are.
    pub noun: Noun,
    /// Number of domains, shown as `across N domains`; `None` hides it.
    pub domains: Option<u32>,
}

const BOLD: &str = "\x1b[1m";
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const CYAN: &str = "\x1b[36m";
const RESET: &str = "\x1b[0m";

/// Braille spinner frames; the last entry is indicatif's "finished" frame.
const SPINNER: [&str; 11] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", " "];

const TICK: Duration = Duration::from_millis(80);

#[derive(Debug, Default)]
struct State {
    header: Option<Header>,
    done: u32,
    failed: u32,
    /// Repos summed from `ok(Some(_))`; `None` until one arrives, so
    /// displays that never count repos (`sync`) leave them out.
    repos: Option<u64>,
    warnings: Vec<String>,
    /// Every bar added to the display, so the end can remove them all
    /// (failed bars must stay alive to keep their `✗` line on screen).
    bars: Vec<ProgressBar>,
}

impl State {
    fn finished(&self) -> u32 {
        self.done.saturating_add(self.failed)
    }
}

#[derive(Debug)]
struct Shared {
    multi: MultiProgress,
    header: ProgressBar,
    title: String,
    color: bool,
    state: Mutex<State>,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        // The state stays consistent across a panicking holder: every
        // update is a single field write.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn paint(&self, style: &str, text: &str) -> String {
        if self.color {
            format!("{style}{text}{RESET}")
        } else {
            text.to_owned()
        }
    }

    /// Redraws the header: the title, or `Updating 12 sources  [7/12]`.
    fn refresh_header(&self, state: &State) {
        let text = match &state.header {
            Some(header) => {
                let across = match header.domains {
                    Some(1) => " across 1 domain".to_owned(),
                    Some(d) => format!(" across {d} domains"),
                    None => String::new(),
                };
                format!(
                    "{} {} {}{across}  [{}/{}]",
                    header.verb.word(),
                    header.total,
                    header.noun.word(header.total),
                    state.finished(),
                    header.total
                )
            }
            None => self.title.clone(),
        };
        self.header.set_message(self.paint(BOLD, &text));
    }
}

/// Lets workers start tasks and queue warnings without writing to the
/// terminal. `Sync`, so scoped workers share `&Tasks`.
#[derive(Debug)]
pub struct Tasks {
    shared: Arc<Shared>,
}

impl Tasks {
    /// Tasks drawn on `multi`, showing `title` until [`Tasks::set_total`].
    pub(crate) fn new(multi: MultiProgress, title: &str, color: bool) -> Self {
        let header = multi.add(ProgressBar::new(0).with_style(plain_style()));
        let shared = Shared {
            multi,
            header,
            title: one_line(title),
            color,
            state: Mutex::new(State::default()),
        };
        let state = shared.state();
        shared.refresh_header(&state);
        drop(state);
        shared.header.tick();
        Self {
            shared: Arc::new(shared),
        }
    }

    /// Replaces the title with a counted header, e.g.
    /// `Updating 12 sources across 2 domains  [7/12]`. Call once, before
    /// any task starts.
    pub fn set_total(&self, verb: Verb, total: u32, noun: Noun, domains: Option<u32>) {
        let header = Header {
            verb,
            total,
            noun,
            domains,
        };
        self.shared.header.set_length(u64::from(total));
        let mut state = self.shared.state();
        state.header = Some(header);
        self.shared.refresh_header(&state);
    }

    /// Starts a task shown as a spinner line `label` (e.g.
    /// `github.com/acme`).
    pub fn start(&self, label: &str) -> Task {
        let shared = &self.shared;
        let label = one_line(label);
        let frames: Vec<String> = SPINNER.iter().map(|f| shared.paint(CYAN, f)).collect();
        let frames: Vec<&str> = frames.iter().map(String::as_str).collect();
        let style = ProgressStyle::with_template("  {spinner} {wide_msg}")
            .map_or_else(|_| plain_style(), |style| style.tick_strings(&frames));
        let bar = shared
            .multi
            .add(ProgressBar::new_spinner().with_style(style));
        bar.set_message(label.clone());
        if !shared.multi.is_hidden() {
            bar.enable_steady_tick(TICK);
        }
        shared.state().bars.push(bar.clone());
        Task {
            label,
            bar,
            shared: Arc::clone(shared),
            finished: false,
        }
    }

    /// Queues a one-line warning, printed after the display is cleared.
    pub fn warn(&self, message: impl Into<String>) {
        let message = message.into();
        self.shared.state().warnings.push(message);
    }

    /// Ends the display: removes every bar (clearing it from the screen
    /// unless the thread is panicking, when a stale frame is left), and
    /// returns the summary line (`None` if nothing finished) and the
    /// deferred warnings.
    pub(crate) fn end(&self) -> (Option<String>, Vec<String>) {
        let shared = &self.shared;
        let mut state = shared.state();
        // Best effort: a broken stderr loses the display.
        let _ = shared.multi.clear();
        shared.multi.remove(&shared.header);
        for bar in mem::take(&mut state.bars) {
            shared.multi.remove(&bar);
        }
        (self.summary_line(&state), mem::take(&mut state.warnings))
    }

    /// `✓ 12 sources · 1,204 repos · 1 failed`; `None` if nothing ran.
    fn summary_line(&self, state: &State) -> Option<String> {
        let finished = state.finished();
        if finished == 0 {
            return None;
        }
        let count = group(u64::from(finished));
        let mut parts = vec![match &state.header {
            Some(header) => format!("{count} {}", header.noun.word(finished)),
            None => format!("{count} finished"),
        }];
        if let Some(repos) = state.repos {
            let unit = plural(repos, "repo", "repos");
            parts.push(format!("{} {unit}", group(repos)));
        }
        if state.failed > 0 {
            parts.push(format!("{} failed", state.failed));
        }
        let glyph = if state.done == 0 {
            self.shared.paint(RED, "✗")
        } else {
            self.shared.paint(GREEN, "✓")
        };
        Some(format!("{glyph} {}", parts.join(" · ")))
    }
}

fn plain_style() -> ProgressStyle {
    ProgressStyle::with_template("{wide_msg}").unwrap_or_else(|_| ProgressStyle::default_bar())
}

/// One in-flight task. End it with [`Task::ok`] or [`Task::fail`]; a task
/// dropped without either (an early return, or a panicking worker) ends as
/// failed with the reason `abandoned`, so the display always completes.
#[derive(Debug)]
#[must_use = "end the task with `ok` or `fail`"]
pub struct Task {
    label: String,
    bar: ProgressBar,
    shared: Arc<Shared>,
    finished: bool,
}

impl Task {
    /// Replaces the text shown after the task's label (e.g. `280 repos`).
    pub fn detail(&self, text: &str) {
        let text = one_line(text);
        self.bar.set_message(format!("{}  {text}", self.label));
    }

    /// Ends the task successfully and clears its line; `repos` (when known)
    /// is summed into the final summary line.
    pub fn ok(mut self, repos: Option<u32>) {
        self.finished = true;
        self.bar.finish_and_clear();
        self.shared.multi.remove(&self.bar);
        let mut state = self.shared.state();
        state.done = state.done.saturating_add(1);
        if let Some(n) = repos {
            state.repos = Some(state.repos.unwrap_or(0).saturating_add(u64::from(n)));
        }
        self.shared.refresh_header(&state);
        drop(state);
    }

    /// Ends the task with a failure, kept on screen as
    /// `✗ <label>  <reason>` until the display ends.
    pub fn fail(mut self, reason: &str) {
        self.finish_failed(reason);
    }

    fn finish_failed(&mut self, reason: &str) {
        self.finished = true;
        let reason = one_line(reason);
        let glyph = self.shared.paint(RED, "✗");
        self.bar.set_style(
            ProgressStyle::with_template("  {wide_msg}").unwrap_or_else(|_| plain_style()),
        );
        self.bar
            .abandon_with_message(format!("{glyph} {}  {reason}", self.label));
        let mut state = self.shared.state();
        state.failed = state.failed.saturating_add(1);
        self.shared.refresh_header(&state);
        drop(state);
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        if !self.finished {
            self.finish_failed("abandoned");
        }
    }
}

/// `1204` → `1,204`.
#[must_use]
pub(crate) fn group(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}
