//! Progress for long-running work: workers report through [`Tasks`] and
//! [`Task`], which drive `indicatif` bars on the `MultiProgress` that
//! `ui::Ui` owns. Workers never write to the terminal; `ui::Ui::with_progress`
//! prints the summary line and the deferred warnings once the display ends.

use std::mem;
#[cfg(test)]
use std::sync::atomic::{AtomicU32, Ordering};
#[cfg(test)]
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

#[cfg(test)]
use indicatif::ProgressDrawTarget;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

use crate::text::{one_line, plural};

/// What a progress display counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Noun {
    /// Repo listings (`update`): each domain's personal listing plus one
    /// per selected org.
    Sources,
    /// Repositories (`sync`).
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
    /// `update` and the automatic refresh.
    Updating,
    /// `sync`.
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
    /// `git fetch` (sync), shown as `fetching…`.
    Fetching,
    /// `git merge --ff-only` (sync), shown as `fast-forwarding`.
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

/// ANSI styles, used only when colour is on.
const BOLD: &str = "\x1b[1m";
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const CYAN: &str = "\x1b[36m";
const RESET: &str = "\x1b[0m";

/// Braille spinner frames; the last entry is indicatif's "finished" frame.
const SPINNER: [&str; 11] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", " "];

/// How often a running task's spinner advances.
const TICK: Duration = Duration::from_millis(80);

/// Counters and deferred output shared by [`Tasks`] and every [`Task`].
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
    #[cfg(test)]
    next_id: AtomicU32,
    /// Test-only recording of what workers reported.
    #[cfg(test)]
    events: Option<mpsc::Sender<UiEvent>>,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        // The state stays consistent across a panicking holder: every
        // update is a single field write.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[cfg(test)]
    fn record(&self, event: UiEvent) {
        if let Some(tx) = &self.events {
            // A closed receiver means the test stopped listening.
            tx.send(event).ok();
        }
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

/// What workers get from `ui::Ui::with_progress`: they start tasks and
/// queue warnings, and never write to the terminal themselves. `Sync`, so
/// scoped workers share `&Tasks`.
#[derive(Debug)]
pub struct Tasks {
    shared: Arc<Shared>,
}

impl Tasks {
    /// Tasks drawn on `multi`, showing `title` until [`Tasks::set_total`].
    /// `color` is false under `NO_COLOR` or in plain mode.
    pub(crate) fn new(multi: MultiProgress, title: &str, color: bool) -> Self {
        let header = multi.add(ProgressBar::new(0).with_style(plain_style()));
        let shared = Shared {
            multi,
            header,
            title: one_line(title),
            color,
            state: Mutex::new(State::default()),
            #[cfg(test)]
            next_id: AtomicU32::new(0),
            #[cfg(test)]
            events: None,
        };
        let state = shared.state();
        shared.refresh_header(&state);
        drop(state);
        shared.header.tick();
        Self {
            shared: Arc::new(shared),
        }
    }

    /// Tasks on a hidden display and the receiver of everything they
    /// report, with task ids in start order (tests).
    #[cfg(test)]
    #[must_use]
    pub fn channel() -> (Self, mpsc::Receiver<UiEvent>) {
        let (tx, rx) = mpsc::channel();
        let mut tasks = Self::new(
            MultiProgress::with_draw_target(ProgressDrawTarget::hidden()),
            "",
            false,
        );
        if let Some(shared) = Arc::get_mut(&mut tasks.shared) {
            shared.events = Some(tx);
        }
        (tasks, rx)
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
        #[cfg(test)]
        self.shared.record(UiEvent::Header(header.clone()));
        let mut state = self.shared.state();
        state.header = Some(header);
        self.shared.refresh_header(&state);
    }

    /// Starts a task shown as a spinner line `label` (e.g.
    /// `github.com/acme`).
    pub fn start(&self, label: &str) -> Task {
        let shared = &self.shared;
        #[cfg(test)]
        let id = shared.next_id.fetch_add(1, Ordering::Relaxed);
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
        #[cfg(test)]
        shared.record(UiEvent::TaskStarted {
            id,
            label: label.clone(),
        });
        Task {
            #[cfg(test)]
            id,
            label,
            bar,
            shared: Arc::clone(shared),
            finished: false,
        }
    }

    /// Queues a one-line warning, printed after the display is cleared.
    pub fn warn(&self, message: impl Into<String>) {
        let message = message.into();
        #[cfg(test)]
        self.shared.record(UiEvent::Warn(message.clone()));
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

/// A bare `{wide_msg}` style.
fn plain_style() -> ProgressStyle {
    ProgressStyle::with_template("{wide_msg}").unwrap_or_else(|_| ProgressStyle::default_bar())
}

/// One in-flight task. End it with [`Task::ok`] or [`Task::fail`]; a task
/// dropped without either (an early return, or a panicking worker) ends as
/// failed with the reason `abandoned`, so the display always completes.
#[derive(Debug)]
#[must_use = "end the task with `ok` or `fail`"]
pub struct Task {
    #[cfg(test)]
    id: u32,
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
        #[cfg(test)]
        self.shared.record(UiEvent::TaskProgress {
            id: self.id,
            detail: TaskDetail::Text(text),
        });
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
        #[cfg(test)]
        self.shared.record(UiEvent::TaskFinished {
            id: self.id,
            outcome: TaskOutcome::Ok { repos },
        });
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
        #[cfg(test)]
        self.shared.record(UiEvent::TaskFinished {
            id: self.id,
            outcome: TaskOutcome::Failed { reason },
        });
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

/// The in-flight detail a worker reported (tests).
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskDetail {
    /// Free text, from [`Task::detail`], sanitised.
    Text(String),
}

/// How a task ended (tests).
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskOutcome {
    /// [`Task::ok`].
    Ok {
        /// Repos fetched, when known.
        repos: Option<u32>,
    },
    /// [`Task::fail`], or `abandoned` when dropped unfinished.
    Failed {
        /// One line, sanitised.
        reason: String,
    },
}

/// One thing a worker reported through [`Tasks`] (tests; see
/// [`Tasks::channel`]). Ids are assigned in start order from 0.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiEvent {
    /// [`Tasks::set_total`].
    Header(Header),
    /// [`Tasks::start`].
    TaskStarted {
        /// Task id.
        id: u32,
        /// What the task works on, sanitised.
        label: String,
    },
    /// [`Task::detail`].
    TaskProgress {
        /// Task id.
        id: u32,
        /// The new detail.
        detail: TaskDetail,
    },
    /// [`Task::ok`] or [`Task::fail`].
    TaskFinished {
        /// Task id.
        id: u32,
        /// How it ended.
        outcome: TaskOutcome,
    },
    /// [`Tasks::warn`].
    Warn(String),
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::io;
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

    use indicatif::{ProgressDrawTarget, TermLike};

    use super::{Noun, TaskOutcome, Tasks, UiEvent, Verb};
    use crate::testing::{ScriptedInteraction, SharedBuf};
    use crate::ui::{Mode, Ui};

    const WIDTH: u16 = 60;

    /// A tiny terminal: autowrapping rows, a cursor, and every string
    /// indicatif wrote.
    #[derive(Debug, Default)]
    struct Screen {
        rows: Vec<Vec<char>>,
        row: usize,
        col: usize,
        writes: Vec<String>,
    }

    impl Screen {
        fn put(&mut self, s: &str) {
            self.writes.push(s.to_owned());
            for c in s.chars() {
                if c == '\r' {
                    self.col = 0;
                    continue;
                }
                if self.col >= usize::from(WIDTH) {
                    self.row += 1;
                    self.col = 0;
                }
                if self.rows.len() <= self.row {
                    self.rows.resize(self.row + 1, Vec::new());
                }
                let line = &mut self.rows[self.row];
                if line.len() <= self.col {
                    line.resize(self.col + 1, ' ');
                }
                line[self.col] = c;
                self.col += 1;
            }
        }

        /// The non-blank rows, right-trimmed.
        fn visible(&self) -> Vec<String> {
            self.rows
                .iter()
                .map(|r| r.iter().collect::<String>().trim_end().to_owned())
                .filter(|r| !r.is_empty())
                .collect()
        }
    }

    #[derive(Debug, Clone, Default)]
    struct FakeTerm(Arc<Mutex<Screen>>);

    impl FakeTerm {
        fn screen(&self) -> MutexGuard<'_, Screen> {
            self.0.lock().unwrap_or_else(PoisonError::into_inner)
        }
    }

    impl TermLike for FakeTerm {
        fn width(&self) -> u16 {
            WIDTH
        }
        fn height(&self) -> u16 {
            24
        }
        fn move_cursor_up(&self, n: usize) -> io::Result<()> {
            let mut s = self.screen();
            s.row = s.row.saturating_sub(n);
            Ok(())
        }
        fn move_cursor_down(&self, n: usize) -> io::Result<()> {
            self.screen().row += n;
            Ok(())
        }
        fn move_cursor_right(&self, n: usize) -> io::Result<()> {
            self.screen().col += n;
            Ok(())
        }
        fn move_cursor_left(&self, n: usize) -> io::Result<()> {
            let mut s = self.screen();
            s.col = s.col.saturating_sub(n);
            Ok(())
        }
        fn write_line(&self, s: &str) -> io::Result<()> {
            let mut screen = self.screen();
            screen.put(s);
            screen.row += 1;
            screen.col = 0;
            Ok(())
        }
        fn write_str(&self, s: &str) -> io::Result<()> {
            self.screen().put(s);
            Ok(())
        }
        fn clear_line(&self) -> io::Result<()> {
            let mut s = self.screen();
            let row = s.row;
            if let Some(line) = s.rows.get_mut(row) {
                line.clear();
            }
            s.col = 0;
            Ok(())
        }
        fn flush(&self) -> io::Result<()> {
            Ok(())
        }
    }

    /// A Ui in `mode`; in `Tty` mode live progress draws on the returned
    /// [`FakeTerm`].
    fn ui(mode: Mode) -> (Ui, SharedBuf, SharedBuf, FakeTerm) {
        let (out, err, term) = (SharedBuf::new(), SharedBuf::new(), FakeTerm::default());
        let mut ui = Ui::for_test_mode(
            mode,
            Box::new(out.clone()),
            Box::new(err.clone()),
            Box::new(ScriptedInteraction::new(false, vec![])),
        );
        if mode == Mode::Tty {
            ui.set_progress_target(ProgressDrawTarget::term_like(Box::new(term.clone())));
        }
        (ui, out, err, term)
    }

    /// Two orgs: one fetches 1,204 repos, one fails; one warning. Returns
    /// the screen as it was once both had ended.
    fn two_orgs(tasks: &Tasks, term: &FakeTerm) -> Vec<String> {
        tasks.set_total(Verb::Updating, 2, Noun::Sources, Some(1));
        let acme = tasks.start("github.com/acme");
        let widgets = tasks.start("github.com/widgets");
        acme.detail("page 3");
        tasks.warn("org widgets failed: HTTP 502");
        acme.ok(Some(1_204));
        widgets.fail("HTTP 502");
        term.screen().visible()
    }

    #[test]
    fn tty_mode_keeps_failures_on_screen_then_prints_the_summary_and_warnings() {
        let (mut ui, out, err, term) = ui(Mode::Tty);
        let during = ui.with_progress("Updating…", |tasks| two_orgs(tasks, &term));
        assert!(
            during
                .iter()
                .any(|row| row == "  ✗ github.com/widgets  HTTP 502"),
            "{during:?}"
        );
        assert!(!during.iter().any(|row| row.contains("acme")), "{during:?}");
        assert_eq!(term.screen().visible(), Vec::<String>::new(), "cleared");
        assert_eq!(out.contents(), "");
        assert_eq!(
            err.contents(),
            "✓ 2 sources · 1,204 repos · 1 failed\nwarning: org widgets failed: HTTP 502\n"
        );
    }

    #[test]
    fn plain_mode_prints_only_the_warnings() {
        let (mut ui, out, err, term) = ui(Mode::Plain);
        ui.with_progress("Updating…", |tasks| two_orgs(tasks, &term));
        assert_eq!(out.contents(), "");
        assert_eq!(err.contents(), "warning: org widgets failed: HTTP 502\n");
        assert_eq!(term.screen().writes, Vec::<String>::new());
    }

    #[test]
    fn a_task_dropped_without_an_outcome_ends_as_abandoned() {
        let (tasks, rx) = Tasks::channel();
        drop(tasks.start("github.com/acme"));
        let events: Vec<UiEvent> = rx.try_iter().collect();
        assert_matches!(
            events.last(),
            Some(UiEvent::TaskFinished { id: 0, outcome: TaskOutcome::Failed { reason } })
                if reason == "abandoned"
        );

        let (mut ui, _out, err, _term) = ui(Mode::Tty);
        ui.with_progress("t", |tasks| drop(tasks.start("acme/a")));
        assert_eq!(err.contents(), "✗ 1 finished · 1 failed\n");
    }

    #[test]
    fn control_characters_never_reach_the_terminal() {
        let (mut ui, _out, _err, term) = ui(Mode::Tty);
        ui.with_progress("a\nb", |tasks| {
            let task = tasks.start("acme\x1b[2J");
            task.detail("page 1\r\nboom");
            task.fail("bad\nreason");
        });
        let screen = term.screen();
        assert_ne!(screen.writes, Vec::<String>::new());
        for write in screen.writes.iter().filter(|w| *w != "\r") {
            assert!(!write.contains(char::is_control), "{write:?}");
        }
    }
}
