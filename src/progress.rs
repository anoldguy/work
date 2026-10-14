//! Progress events from workers to the terminal owner, and the pure view
//! that folds them into display lines.
//!
//! This module has no terminal dependencies. Workers only ever hold a
//! [`ProgressSink`]; `ui::Ui::with_progress` owns the receiver, folds events
//! into a [`ProgressView`], and draws it.

use std::collections::BTreeMap;
use std::sync::mpsc;

/// What a progress display counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Noun {
    /// Org jobs (`update`); personal jobs count as one each.
    Orgs,
    /// Repositories (`sync`).
    Repositories,
}

impl Noun {
    /// The word for `n` items: `org`/`orgs`, `repository`/`repositories`.
    #[must_use]
    pub const fn word(self, n: u32) -> &'static str {
        match (self, n) {
            (Self::Orgs, 1) => "org",
            (Self::Orgs, _) => "orgs",
            (Self::Repositories, 1) => "repository",
            (Self::Repositories, _) => "repositories",
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
/// `Updating 12 orgs across 2 domains  [7/12]` or
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

/// The in-flight detail shown after a task's label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskDetail {
    /// `page 3 · 280 repos`.
    Page {
        /// Pages received so far.
        page: u32,
        /// Repos received so far.
        repos: u32,
    },
    /// A fixed step.
    Step(Step),
}

/// How a task ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskOutcome {
    /// Success; collapses into the `✓ N done` line.
    Ok {
        /// Repos fetched, summed into the final summary line; `None` for
        /// tasks that do not fetch repos.
        repos: Option<u32>,
    },
    /// Failure; keeps a `✗ <label>  <reason>` line until the end.
    Failed {
        /// One line, already redacted.
        reason: String,
    },
}

/// One message from a worker to the terminal owner.
///
/// Task ids are chosen by the sender and are unique within one
/// `with_progress` call (the job's index is the natural choice).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiEvent {
    /// Replaces the title line with a counted header. Sent once, before any
    /// task starts.
    Header(Header),
    /// A task began; `label` is e.g. `github.com/acme` or `acme/foo`.
    TaskStarted {
        /// Task id.
        id: u32,
        /// What the task works on.
        label: String,
    },
    /// A task advanced.
    TaskProgress {
        /// Task id.
        id: u32,
        /// The new detail.
        detail: TaskDetail,
    },
    /// A task ended.
    TaskFinished {
        /// Task id.
        id: u32,
        /// How it ended.
        outcome: TaskOutcome,
    },
    /// A warning, printed (one line) after the display is cleared.
    Warn(String),
}

/// The sending half given to workers. Cheap to clone and `Sync`, so scoped
/// workers can share `&ProgressSink`. Sends never fail: once the receiver is
/// gone, events are dropped.
#[derive(Debug, Clone)]
pub struct ProgressSink(mpsc::Sender<UiEvent>);

impl ProgressSink {
    /// A connected sink and receiver.
    #[must_use]
    pub fn channel() -> (Self, mpsc::Receiver<UiEvent>) {
        let (tx, rx) = mpsc::channel();
        (Self(tx), rx)
    }

    /// Sends `event`, ignoring a disconnected receiver.
    pub fn send(&self, event: UiEvent) {
        // A closed receiver means nobody is drawing any more; dropping the
        // event is the intended behaviour.
        self.0.send(event).ok();
    }

    /// Sends a [`UiEvent::Warn`].
    pub fn warn(&self, message: impl Into<String>) {
        self.send(UiEvent::Warn(message.into()));
    }
}

/// One in-flight task in a [`ProgressView`].
#[derive(Debug, Clone, PartialEq, Eq)]
struct InFlight {
    label: String,
    detail: Option<TaskDetail>,
}

/// One failed task in a [`ProgressView`].
#[derive(Debug, Clone, PartialEq, Eq)]
struct FailedTask {
    label: String,
    reason: String,
}

/// Braille spinner frames; in-flight line `i` shows frame `tick + i`.
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// ANSI styles used when colour is on.
const BOLD: &str = "\x1b[1m";
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const CYAN: &str = "\x1b[36m";
const RESET: &str = "\x1b[0m";

/// The pure state of the live multi-line display (snapshot-tested).
///
/// Layout, top to bottom: the header (or the title until a header arrives)
/// with `[done/total]`; `✓ N done · M failed`; one `✗` line per failure; one
/// spinner line per in-flight task, capped to the height with
/// `… and K more`. Warnings are not part of the view.
///
/// Text from events and the title is stored with control characters
/// replaced by spaces, so a stray `\n` or `\r` cannot add terminal rows.
#[derive(Debug, Default)]
pub struct ProgressView {
    title: String,
    color: bool,
    header: Option<Header>,
    in_flight: BTreeMap<u32, InFlight>,
    done: u32,
    /// Repos summed from `Ok { repos: Some(_) }`; `None` until one arrives,
    /// so displays that never count repos (`sync`) leave them out.
    repos: Option<u64>,
    failed: Vec<FailedTask>,
}

impl ProgressView {
    /// An empty view showing `title` until a `Header` arrives. `color` is
    /// false under `NO_COLOR` or in plain mode.
    #[must_use]
    pub fn new(title: &str, color: bool) -> Self {
        Self {
            title: sanitize(title),
            color,
            ..Self::default()
        }
    }

    /// Folds one event in. `Warn` events are ignored here; the owner defers
    /// them.
    pub fn apply(&mut self, event: UiEvent) {
        match event {
            UiEvent::Header(header) => self.header = Some(header),
            UiEvent::TaskStarted { id, label } => {
                self.in_flight.insert(
                    id,
                    InFlight {
                        label: sanitize(&label),
                        detail: None,
                    },
                );
            }
            UiEvent::TaskProgress { id, detail } => {
                if let Some(task) = self.in_flight.get_mut(&id) {
                    task.detail = Some(detail);
                }
            }
            UiEvent::TaskFinished { id, outcome } => {
                let label = self
                    .in_flight
                    .remove(&id)
                    .map_or_else(|| format!("task {id}"), |task| task.label);
                match outcome {
                    TaskOutcome::Ok { repos } => {
                        self.done = self.done.saturating_add(1);
                        if let Some(n) = repos {
                            self.repos = Some(self.repos.unwrap_or(0).saturating_add(u64::from(n)));
                        }
                    }
                    TaskOutcome::Failed { reason } => {
                        self.failed.push(FailedTask {
                            label,
                            reason: sanitize(&reason),
                        });
                    }
                }
            }
            UiEvent::Warn(_) => {}
        }
    }

    /// Finished tasks, successful or not.
    fn finished(&self) -> u32 {
        let failed = u32::try_from(self.failed.len()).unwrap_or(u32::MAX);
        self.done.saturating_add(failed)
    }

    /// The display for a `w`×`h` terminal at spinner frame `tick`.
    ///
    /// Every line is at most `w - 1` columns wide (see [`display_width`])
    /// and there are at most `h - 1` lines (never fewer than one), so an
    /// in-place redraw never wraps or scrolls. When there is no row left
    /// for task lines, the in-flight count joins the count line.
    #[must_use]
    pub fn lines(&self, w: u16, h: u16, tick: u8) -> Vec<String> {
        let width = usize::from(w.saturating_sub(1).max(1));
        let mut out = vec![self.head_line(width)];
        // Rows below the header; one row is always left for the cursor.
        let mut rows = usize::from(h.saturating_sub(2));
        if rows == 0 {
            return out;
        }

        let running = self.in_flight.len();
        let show_count = self.finished() > 0;
        if show_count {
            rows -= 1;
        }
        let failed_shown = self.failed.len().min(rows);
        rows -= failed_shown;
        let (in_flight_shown, more_line) = if running <= rows {
            (running, false)
        } else if rows >= 1 {
            (rows - 1, true)
        } else {
            (0, false)
        };
        // No row at all for in-flight tasks: say how many in the count line.
        let fold_running = running > in_flight_shown && !more_line;

        if show_count || fold_running {
            let mut parts = vec![format!("{} done", self.done)];
            if !self.failed.is_empty() {
                parts.push(format!("{} failed", self.failed.len()));
            }
            if fold_running {
                parts.push(format!("{running} running"));
            }
            out.push(self.row("✓", GREEN, &parts.join(" · "), width));
        }

        let failed = &self.failed[..failed_shown];
        let pad = failed
            .iter()
            .map(|f| display_width(&f.label))
            .chain(
                self.in_flight
                    .values()
                    .take(in_flight_shown)
                    .map(|t| display_width(&t.label)),
            )
            .max()
            .unwrap_or(0);

        for f in failed {
            let text = format!("{}  {}", pad_to(&f.label, pad), f.reason);
            out.push(self.row("✗", RED, &text, width));
        }
        for (i, task) in self.in_flight.values().take(in_flight_shown).enumerate() {
            let frame = SPINNER[(usize::from(tick) + i) % SPINNER.len()];
            let text = match &task.detail {
                Some(detail) => format!("{}  {}", pad_to(&task.label, pad), detail_text(detail)),
                None => task.label.clone(),
            };
            out.push(self.row(frame, CYAN, &text, width));
        }
        if more_line {
            let more = running - in_flight_shown;
            out.push(truncate(&format!("  … and {more} more"), width));
        }
        out
    }

    /// The header (with `[done/total]`) or, before one arrives, the title.
    fn head_line(&self, width: usize) -> String {
        let text = match &self.header {
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
                    self.finished(),
                    header.total
                )
            }
            None => self.title.clone(),
        };
        let text = truncate(&text, width);
        if self.color {
            format!("{BOLD}{text}{RESET}")
        } else {
            text
        }
    }

    /// One `  <glyph> <text>` row, truncated to `width`, glyph coloured.
    fn row(&self, glyph: &str, style: &str, text: &str, width: usize) -> String {
        let plain = truncate(&format!("  {glyph} {text}"), width);
        let prefix = format!("  {glyph}");
        match plain.strip_prefix(&prefix) {
            Some(rest) if self.color => format!("  {style}{glyph}{RESET}{rest}"),
            _ => plain,
        }
    }

    /// The line left on stderr once the display is cleared, e.g.
    /// `✓ 12 orgs · 1,204 repos · 1 failed`. `None` if nothing ran.
    #[must_use]
    pub fn summary_line(&self) -> Option<String> {
        let finished = self.finished();
        if finished == 0 {
            return None;
        }
        let count = group(u64::from(finished));
        let mut parts = vec![match &self.header {
            Some(header) => format!("{count} {}", header.noun.word(finished)),
            None => format!("{count} finished"),
        }];
        if let Some(repos) = self.repos {
            let unit = if repos == 1 { "repo" } else { "repos" };
            parts.push(format!("{} {unit}", group(repos)));
        }
        if !self.failed.is_empty() {
            parts.push(format!("{} failed", self.failed.len()));
        }
        let (glyph, style) = if self.done == 0 {
            ("✗", RED)
        } else {
            ("✓", GREEN)
        };
        let glyph = if self.color {
            format!("{style}{glyph}{RESET}")
        } else {
            glyph.to_owned()
        };
        Some(format!("{glyph} {}", parts.join(" · ")))
    }
}

/// The in-flight text for a detail: `page 3 · 280 repos` or a step label.
fn detail_text(detail: &TaskDetail) -> String {
    match detail {
        TaskDetail::Page { page, repos } => {
            let unit = if *repos == 1 { "repo" } else { "repos" };
            format!("page {page} · {} {unit}", group(u64::from(*repos)))
        }
        TaskDetail::Step(step) => step.label().to_owned(),
    }
}

/// `1204` → `1,204`.
fn group(n: u64) -> String {
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

/// `text` with every control character replaced by a space.
fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Conservative terminal columns for `c`: never less than the terminal
/// will use, so a line measured with it cannot wrap.
///
/// ASCII, the view's own narrow glyphs (`✓`, `✗`, braille spinners) and
/// Latin/Greek/Cyrillic letters are one column; combining marks are zero;
/// everything else (CJK, emoji, and East Asian "ambiguous" symbols such as
/// `…` and `·`) counts as two.
const fn char_width(c: char) -> usize {
    match c {
        '\u{0300}'..='\u{036F}' => 0,
        '×' | '÷' => 2,
        ' '..='~'
        | '✓'
        | '✗'
        | '\u{2800}'..='\u{28FF}'
        | 'À'..='ÿ'
        | '\u{0100}'..='\u{02AF}'
        | '\u{0391}'..='\u{03C9}'
        | '\u{0400}'..='\u{04FF}' => 1,
        _ => 2,
    }
}

/// The conservative display width of `text` (see [`char_width`]).
fn display_width(text: &str) -> usize {
    text.chars().map(char_width).sum()
}

/// `text` padded with spaces to `width` display columns.
fn pad_to(text: &str, width: usize) -> String {
    let fill = width.saturating_sub(display_width(text));
    format!("{text}{}", " ".repeat(fill))
}

/// At most `width` display columns, ending in `…` when cut; trailing spaces
/// dropped.
fn truncate(text: &str, width: usize) -> String {
    let text = text.trim_end();
    if display_width(text) <= width {
        return text.to_owned();
    }
    let budget = width.saturating_sub(char_width('…'));
    let mut used = 0;
    let mut cut: String = text
        .chars()
        .take_while(|&c| {
            used += char_width(c);
            used <= budget
        })
        .collect();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::{
        Header, Noun, ProgressSink, ProgressView, Step, TaskDetail, TaskOutcome, UiEvent, Verb,
        display_width,
    };

    fn update_header(total: u32) -> UiEvent {
        UiEvent::Header(Header {
            verb: Verb::Updating,
            total,
            noun: Noun::Orgs,
            domains: Some(2),
        })
    }

    fn sync_header(total: u32) -> UiEvent {
        UiEvent::Header(Header {
            verb: Verb::Syncing,
            total,
            noun: Noun::Repositories,
            domains: None,
        })
    }

    fn start(view: &mut ProgressView, id: u32, label: &str) {
        view.apply(UiEvent::TaskStarted {
            id,
            label: label.to_owned(),
        });
    }

    fn page(view: &mut ProgressView, id: u32, page: u32, repos: u32) {
        view.apply(UiEvent::TaskProgress {
            id,
            detail: TaskDetail::Page { page, repos },
        });
    }

    fn ok(view: &mut ProgressView, id: u32, repos: Option<u32>) {
        view.apply(UiEvent::TaskFinished {
            id,
            outcome: TaskOutcome::Ok { repos },
        });
    }

    fn fail(view: &mut ProgressView, id: u32, reason: &str) {
        view.apply(UiEvent::TaskFinished {
            id,
            outcome: TaskOutcome::Failed {
                reason: reason.to_owned(),
            },
        });
    }

    /// The §5.4 example: 12 orgs, 6 done, 1 failed, 3 in flight.
    fn example(color: bool) -> ProgressView {
        let mut view = ProgressView::new("Updating…", color);
        view.apply(update_header(12));
        for id in 0..6 {
            start(&mut view, id, &format!("github.com/org{id}"));
            ok(&mut view, id, Some(100));
        }
        start(&mut view, 6, "ghe.corp.com/infra");
        fail(&mut view, 6, "HTTP 502 (kept 41 cached repos)");
        start(&mut view, 7, "github.com/acme");
        page(&mut view, 7, 3, 280);
        start(&mut view, 8, "github.com/widgets");
        page(&mut view, 8, 1, 64);
        start(&mut view, 9, "github.com/@me (personal)");
        page(&mut view, 9, 2, 150);
        view
    }

    /// 1 failure and 10 in-flight repos.
    fn crowded() -> ProgressView {
        let mut view = ProgressView::new("Syncing…", false);
        view.apply(sync_header(20));
        start(&mut view, 99, "acme/broken");
        fail(&mut view, 99, "not a fast-forward");
        for id in 0..10 {
            start(&mut view, id, &format!("acme/repo{id}"));
        }
        view
    }

    fn frame(view: &ProgressView, w: u16, h: u16, tick: u8) -> String {
        view.lines(w, h, tick).join("\n")
    }

    #[test]
    fn shows_the_title_until_a_header_arrives() {
        let view = ProgressView::new("Refreshing cache", false);
        insta::assert_snapshot!(frame(&view, 80, 24, 0), @r"Refreshing cache");
        assert_eq!(view.summary_line(), None);
    }

    #[test]
    fn header_counts_done_over_total() {
        let mut view = ProgressView::new("Updating…", false);
        view.apply(update_header(12));
        insta::assert_snapshot!(frame(&view, 80, 24, 0), @r"Updating 12 orgs across 2 domains  [0/12]");
        let mut sync = ProgressView::new("Syncing…", false);
        sync.apply(sync_header(1));
        insta::assert_snapshot!(frame(&sync, 80, 24, 0), @r"Syncing 1 repository  [0/1]");
    }

    #[test]
    fn in_flight_lines_show_page_and_repo_counts() {
        let mut view = ProgressView::new("Updating…", false);
        view.apply(update_header(2));
        start(&mut view, 0, "github.com/acme");
        page(&mut view, 0, 3, 1280);
        start(&mut view, 1, "acme/foo");
        view.apply(UiEvent::TaskProgress {
            id: 1,
            detail: TaskDetail::Step(Step::Fetching),
        });
        start(&mut view, 2, "acme/bar");
        insta::assert_snapshot!(frame(&view, 80, 24, 1), @r"
            Updating 2 orgs across 2 domains  [0/2]
              ⠙ github.com/acme  page 3 · 1,280 repos
              ⠹ acme/foo         fetching…
              ⠸ acme/bar
        ");
    }

    #[test]
    fn successes_collapse_and_failures_persist() {
        insta::assert_snapshot!(frame(&example(false), 80, 24, 0), @r"
            Updating 12 orgs across 2 domains  [7/12]
              ✓ 6 done · 1 failed
              ✗ ghe.corp.com/infra         HTTP 502 (kept 41 cached repos)
              ⠋ github.com/acme            page 3 · 280 repos
              ⠙ github.com/widgets         page 1 · 64 repos
              ⠹ github.com/@me (personal)  page 2 · 150 repos
        ");
    }

    #[test]
    fn caps_in_flight_lines_to_the_height() {
        let lines = crowded().lines(80, 8, 0);
        assert_eq!(lines.len(), 7, "h - 1 rows: {lines:?}");
        insta::assert_snapshot!(lines.join("\n"), @r"
            Syncing 20 repositories  [1/20]
              ✓ 0 done · 1 failed
              ✗ acme/broken  not a fast-forward
              ⠋ acme/repo0
              ⠙ acme/repo1
              ⠹ acme/repo2
              … and 7 more
        ");
    }

    #[test]
    fn tiny_heights_never_exceed_h_minus_one_rows() {
        let view = crowded();
        for h in 0..=8 {
            let lines = view.lines(80, h, 0);
            assert!(
                lines.len() <= usize::from(h.saturating_sub(1)).max(1),
                "h={h}: {lines:?}"
            );
        }
        insta::assert_snapshot!(frame(&view, 80, 3, 0), @r"
            Syncing 20 repositories  [1/20]
              ✓ 0 done · 1 failed · 10 running
        ");
        insta::assert_snapshot!(frame(&view, 80, 4, 0), @r"
            Syncing 20 repositories  [1/20]
              ✓ 0 done · 1 failed · 10 running
              ✗ acme/broken  not a fast-forward
        ");
        let mut fresh = ProgressView::new("t", false);
        start(&mut fresh, 0, "a");
        start(&mut fresh, 1, "b");
        insta::assert_snapshot!(frame(&fresh, 80, 3, 0), @r"
            t
              … and 2 more
        ");
    }

    #[test]
    fn long_lines_are_truncated_to_the_width() {
        let lines = example(false).lines(30, 24, 0);
        assert!(lines.iter().all(|l| display_width(l) <= 29), "{lines:?}");
        insta::assert_snapshot!(lines.join("\n"), @r"
            Updating 12 orgs across 2 d…
              ✓ 6 done · 1 failed
              ✗ ghe.corp.com/infra     …
              ⠋ github.com/acme        …
              ⠙ github.com/widgets     …
              ⠹ github.com/@me (persona…
        ");
    }

    #[test]
    fn wide_text_is_measured_in_columns() {
        let mut view = ProgressView::new("同期しています", false);
        start(&mut view, 0, "acme/a");
        fail(&mut view, 0, "致命的: リモートリポジトリが見つかりません");
        let lines = view.lines(40, 24, 0);
        assert!(lines.iter().all(|l| display_width(l) <= 39), "{lines:?}");
        insta::assert_snapshot!(lines.join("\n"), @r"
            同期しています
              ✓ 0 done · 1 failed
              ✗ acme/a  致命的: リモートリポジト…
        ");
    }

    #[test]
    fn control_characters_never_reach_the_display() {
        let mut view = ProgressView::new("ti\ttle\n", true);
        start(&mut view, 0, "acme/\ra");
        fail(&mut view, 0, "fatal: x\nhint: y\r\tz\x1b[2J");
        start(&mut view, 1, "acme/b\x07");
        let mut text = view.lines(80, 24, 0).join("\n");
        text.push_str(&view.summary_line().expect("ran"));
        for sgr in ["\x1b[1m", "\x1b[31m", "\x1b[32m", "\x1b[36m", "\x1b[0m"] {
            text = text.replace(sgr, "");
        }
        assert_eq!(text.lines().count(), 4, "{text:?}");
        assert!(
            !text.chars().any(|c| c.is_control() && c != '\n'),
            "{text:?}"
        );
    }

    #[test]
    fn no_color_output_has_no_escapes_and_color_output_does() {
        let plain = frame(&example(false), 80, 24, 0);
        assert!(!plain.contains('\x1b'));
        assert!(!example(false).summary_line().expect("ran").contains('\x1b'));
        insta::assert_snapshot!(frame(&example(true), 80, 24, 0).replace('\x1b', "^["), @r"
            ^[[1mUpdating 12 orgs across 2 domains  [7/12]^[[0m
              ^[[32m✓^[[0m 6 done · 1 failed
              ^[[31m✗^[[0m ghe.corp.com/infra         HTTP 502 (kept 41 cached repos)
              ^[[36m⠋^[[0m github.com/acme            page 3 · 280 repos
              ^[[36m⠙^[[0m github.com/widgets         page 1 · 64 repos
              ^[[36m⠹^[[0m github.com/@me (personal)  page 2 · 150 repos
        ");
    }

    #[test]
    fn summary_line_totals_tasks_repos_and_failures() {
        let mut view = example(false);
        ok(&mut view, 7, Some(1004));
        ok(&mut view, 8, None);
        insta::assert_snapshot!(view.summary_line().expect("ran"), @r"✓ 9 orgs · 1,604 repos · 1 failed");
        insta::assert_snapshot!(
            example(true).summary_line().expect("ran").replace('\x1b', "^["),
            @r"^[[32m✓^[[0m 7 orgs · 600 repos · 1 failed"
        );
    }

    #[test]
    fn summary_line_omits_repos_when_none_were_counted() {
        let mut view = ProgressView::new("Syncing…", false);
        view.apply(sync_header(2));
        start(&mut view, 0, "acme/a");
        ok(&mut view, 0, None);
        start(&mut view, 1, "acme/b");
        fail(&mut view, 1, "dirty");
        insta::assert_snapshot!(view.summary_line().expect("ran"), @r"✓ 2 repositories · 1 failed");
    }

    #[test]
    fn all_failed_summary_without_a_header_uses_a_cross() {
        let mut view = ProgressView::new("x", false);
        start(&mut view, 0, "a");
        fail(&mut view, 0, "boom");
        insta::assert_snapshot!(view.summary_line().expect("ran"), @r"✗ 1 finished · 1 failed");
    }

    #[test]
    fn warnings_and_unknown_ids_do_not_disturb_the_view() {
        let mut view = ProgressView::new("t", false);
        view.apply(UiEvent::Warn("w".to_owned()));
        page(&mut view, 5, 1, 1);
        assert_eq!(view.lines(80, 24, 0), vec!["t".to_owned()]);
        ok(&mut view, 5, Some(1));
        assert_eq!(
            view.summary_line().as_deref(),
            Some("✓ 1 finished · 1 repo")
        );
    }

    #[test]
    fn sink_delivers_events_in_order() {
        let (sink, rx) = ProgressSink::channel();
        sink.send(UiEvent::Warn("a".to_owned()));
        sink.warn("b");
        drop(sink);
        let got: Vec<UiEvent> = rx.iter().collect();
        assert_eq!(
            got,
            vec![UiEvent::Warn("a".to_owned()), UiEvent::Warn("b".to_owned())]
        );
    }

    #[test]
    fn sink_ignores_a_closed_receiver() {
        let (sink, rx) = ProgressSink::channel();
        drop(rx);
        sink.warn("nobody is listening");
    }
}
