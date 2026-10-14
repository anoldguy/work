//! [`ScriptedInteraction`]: a scripted `Interaction` (filled in by W1.8).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use crate::ui::{Interaction, UiError};

/// One scripted reply, consumed in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// Reply to `pick_one` (`None` = cancelled).
    Pick(Option<usize>),
    /// Reply to `pick_many` (`None` = cancelled).
    PickMany(Option<Vec<bool>>),
    /// Reply to `input` (`None` = cancelled).
    Input(Option<String>),
    /// Reply to `confirm`.
    Confirm(bool),
    /// Any method returns `UiError::Interrupted` (Ctrl-C).
    Interrupt,
}

/// The prompts a [`ScriptedInteraction`] was shown, readable after the
/// interaction has been boxed into a `Ui`.
#[derive(Debug, Clone, Default)]
pub struct PromptLog(Arc<Mutex<Vec<String>>>);

impl PromptLog {
    /// Every prompt so far, in order.
    #[must_use]
    pub fn entries(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Appends a prompt.
    pub fn push(&self, prompt: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(prompt.to_owned());
    }
}

/// An `Interaction` that replays [`Answer`]s and logs prompts.
///
/// When not interactive, every method returns
/// `UiError::NotInteractive { what: "test" }`. A reply of the wrong kind, or
/// running out of replies, panics with the prompt.
#[derive(Debug)]
pub struct ScriptedInteraction {
    interactive: bool,
    answers: VecDeque<Answer>,
    log: PromptLog,
}

impl ScriptedInteraction {
    /// A script of `answers`; `interactive` is what `is_interactive` reports.
    #[must_use]
    pub fn new(interactive: bool, answers: Vec<Answer>) -> Self {
        let _ = (interactive, answers);
        unimplemented!("W1.8")
    }

    /// A handle on the prompt log.
    #[must_use]
    pub fn log(&self) -> PromptLog {
        PromptLog(Arc::clone(&self.log.0))
    }
}

impl Interaction for ScriptedInteraction {
    fn is_interactive(&self) -> bool {
        self.interactive
    }

    fn pick_one(&mut self, prompt: &str, items: &[&str]) -> Result<Option<usize>, UiError> {
        let _ = (&mut self.answers, prompt, items);
        unimplemented!("W1.8")
    }

    fn pick_many(
        &mut self,
        prompt: &str,
        items: &[&str],
        selected: &[bool],
    ) -> Result<Option<Vec<bool>>, UiError> {
        let _ = (self, prompt, items, selected);
        unimplemented!("W1.8")
    }

    fn input(&mut self, prompt: &str, default: &str) -> Result<Option<String>, UiError> {
        let _ = (self, prompt, default);
        unimplemented!("W1.8")
    }

    fn confirm(&mut self, prompt: &str, default: bool) -> Result<bool, UiError> {
        let _ = (self, prompt, default);
        unimplemented!("W1.8")
    }
}
