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

/// The pure state of the live multi-line display (snapshot-tested).
///
/// Layout, top to bottom: the header (or the title until a header arrives)
/// with `[done/total]`; `✓ N done · M failed`; one `✗` line per failure; one
/// spinner line per in-flight task, capped to the height with
/// `… and K more`. Warnings are not part of the view.
#[derive(Debug, Default)]
pub struct ProgressView {
    title: String,
    color: bool,
    header: Option<Header>,
    in_flight: BTreeMap<u32, InFlight>,
    done: u32,
    repos: u64,
    failed: Vec<FailedTask>,
}

impl ProgressView {
    /// An empty view showing `title` until a `Header` arrives. `color` is
    /// false under `NO_COLOR` or in plain mode.
    #[must_use]
    pub fn new(title: &str, color: bool) -> Self {
        Self {
            title: title.to_owned(),
            color,
            ..Self::default()
        }
    }

    /// Folds one event in. `Warn` events are ignored here; the owner defers
    /// them.
    pub fn apply(&mut self, event: UiEvent) {
        let _ = (
            &mut self.header,
            &mut self.in_flight,
            &mut self.done,
            &mut self.repos,
            &mut self.failed,
            event,
        );
        unimplemented!("W1.7")
    }

    /// The display for a `w`×`h` terminal at spinner frame `tick`.
    #[must_use]
    pub fn lines(&self, w: u16, h: u16, tick: u8) -> Vec<String> {
        let _ = (&self.title, self.color, w, h, tick);
        unimplemented!("W1.7")
    }

    /// The line left on stderr once the display is cleared, e.g.
    /// `✓ 12 orgs · 1,204 repos · 1 failed`. `None` if nothing ran.
    #[must_use]
    pub fn summary_line(&self) -> Option<String> {
        let _ = self;
        unimplemented!("W1.7")
    }
}

#[cfg(test)]
mod tests {
    use super::{ProgressSink, UiEvent};

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
