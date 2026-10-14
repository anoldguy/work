//! [`ScriptedInteraction`]: a scripted `Interaction`.

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

/// One prompt a [`ScriptedInteraction`] was shown, with what it offered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptCall {
    /// `pick_one`.
    PickOne {
        /// The prompt text.
        prompt: String,
        /// The items offered, in order.
        items: Vec<String>,
    },
    /// `pick_many`.
    PickMany {
        /// The prompt text.
        prompt: String,
        /// The items offered, in order.
        items: Vec<String>,
        /// The preselection.
        selected: Vec<bool>,
    },
    /// `input`.
    Input {
        /// The prompt text.
        prompt: String,
        /// The default offered.
        default: String,
    },
    /// `confirm`.
    Confirm {
        /// The prompt text.
        prompt: String,
        /// The default answer.
        default: bool,
    },
    /// A bare prompt appended with [`PromptLog::push`].
    Prompt {
        /// The prompt text.
        prompt: String,
    },
}

impl PromptCall {
    /// The prompt text.
    #[must_use]
    pub fn prompt(&self) -> &str {
        match self {
            Self::PickOne { prompt, .. }
            | Self::PickMany { prompt, .. }
            | Self::Input { prompt, .. }
            | Self::Confirm { prompt, .. }
            | Self::Prompt { prompt } => prompt,
        }
    }
}

/// The prompts a [`ScriptedInteraction`] was shown, readable after the
/// interaction has been boxed into a `Ui`.
#[derive(Debug, Clone, Default)]
pub struct PromptLog(Arc<Mutex<Vec<PromptCall>>>);

impl PromptLog {
    /// Every prompt's text so far, in order.
    #[must_use]
    pub fn entries(&self) -> Vec<String> {
        self.calls()
            .iter()
            .map(|call| call.prompt().to_owned())
            .collect()
    }

    /// Every call so far, in order, with the items, preselection or default
    /// it offered.
    #[must_use]
    pub fn calls(&self) -> Vec<PromptCall> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Appends a bare prompt ([`PromptCall::Prompt`]).
    pub fn push(&self, prompt: &str) {
        self.record(PromptCall::Prompt {
            prompt: prompt.to_owned(),
        });
    }

    fn record(&self, call: PromptCall) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
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
        Self {
            interactive,
            answers: answers.into(),
            log: PromptLog::default(),
        }
    }

    /// A handle on the prompt log.
    #[must_use]
    pub fn log(&self) -> PromptLog {
        PromptLog(Arc::clone(&self.log.0))
    }

    /// Logs `call` and takes the next reply; errors when not interactive
    /// or when the reply is [`Answer::Interrupt`].
    fn next(&mut self, call: PromptCall) -> Result<Answer, UiError> {
        if !self.interactive {
            return Err(UiError::NotInteractive { what: "test" });
        }
        let prompt = call.prompt().to_owned();
        self.log.record(call);
        match self.answers.pop_front() {
            Some(Answer::Interrupt) => Err(UiError::Interrupted),
            Some(answer) => Ok(answer),
            None => panic!("no scripted answer left for prompt {prompt:?}"),
        }
    }
}

impl Interaction for ScriptedInteraction {
    fn is_interactive(&self) -> bool {
        self.interactive
    }

    fn pick_one(&mut self, prompt: &str, items: &[&str]) -> Result<Option<usize>, UiError> {
        match self.next(PromptCall::PickOne {
            prompt: prompt.to_owned(),
            items: owned(items),
        })? {
            Answer::Pick(pick) => {
                if let Some(index) = pick {
                    assert!(
                        index < items.len(),
                        "scripted pick {index} out of range for prompt {prompt:?}"
                    );
                }
                Ok(pick)
            }
            other => panic!("expected Answer::Pick for prompt {prompt:?}, got {other:?}"),
        }
    }

    fn pick_many(
        &mut self,
        prompt: &str,
        items: &[&str],
        selected: &[bool],
    ) -> Result<Option<Vec<bool>>, UiError> {
        assert_eq!(
            selected.len(),
            items.len(),
            "pick_many selection length for prompt {prompt:?}"
        );
        match self.next(PromptCall::PickMany {
            prompt: prompt.to_owned(),
            items: owned(items),
            selected: selected.to_vec(),
        })? {
            Answer::PickMany(picks) => {
                if let Some(picks) = &picks {
                    assert_eq!(
                        picks.len(),
                        items.len(),
                        "scripted selection length for prompt {prompt:?}"
                    );
                }
                Ok(picks)
            }
            other => panic!("expected Answer::PickMany for prompt {prompt:?}, got {other:?}"),
        }
    }

    fn input(&mut self, prompt: &str, default: &str) -> Result<Option<String>, UiError> {
        match self.next(PromptCall::Input {
            prompt: prompt.to_owned(),
            default: default.to_owned(),
        })? {
            Answer::Input(text) => Ok(text),
            other => panic!("expected Answer::Input for prompt {prompt:?}, got {other:?}"),
        }
    }

    fn confirm(&mut self, prompt: &str, default: bool) -> Result<bool, UiError> {
        match self.next(PromptCall::Confirm {
            prompt: prompt.to_owned(),
            default,
        })? {
            Answer::Confirm(yes) => Ok(yes),
            other => panic!("expected Answer::Confirm for prompt {prompt:?}, got {other:?}"),
        }
    }
}

fn owned(items: &[&str]) -> Vec<String> {
    items.iter().map(|&item| item.to_owned()).collect()
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::{Answer, PromptCall, ScriptedInteraction};
    use crate::ui::{Interaction, UiError};

    #[test]
    fn replays_answers_in_order_and_logs_prompts() {
        let mut script = ScriptedInteraction::new(
            true,
            vec![
                Answer::Pick(Some(1)),
                Answer::PickMany(Some(vec![true, false])),
                Answer::Input(Some("~/src".to_owned())),
                Answer::Confirm(false),
                Answer::Pick(None),
                Answer::Interrupt,
            ],
        );
        let log = script.log();
        assert!(script.is_interactive());
        assert_matches!(script.pick_one("one", &["a", "b"]), Ok(Some(1)));
        assert_eq!(
            script
                .pick_many("many", &["a", "b"], &[false, false])
                .expect("pick_many"),
            Some(vec![true, false])
        );
        assert_eq!(
            script.input("dir", "~/work").expect("input"),
            Some("~/src".to_owned())
        );
        assert_matches!(script.confirm("sure?", true), Ok(false));
        assert_matches!(script.pick_one("again", &["a"]), Ok(None));
        assert_matches!(script.confirm("last", true), Err(UiError::Interrupted));
        assert_eq!(
            log.entries(),
            ["one", "many", "dir", "sure?", "again", "last"]
        );
    }

    #[test]
    fn the_log_records_what_each_prompt_offered() {
        let mut script = ScriptedInteraction::new(
            true,
            vec![
                Answer::Pick(None),
                Answer::PickMany(None),
                Answer::Input(None),
                Answer::Confirm(true),
            ],
        );
        let log = script.log();
        let _ = script.pick_one("one", &["a", "b"]);
        let _ = script.pick_many("many", &["x", "y"], &[true, false]);
        let _ = script.input("dir", "~/work");
        let _ = script.confirm("sure?", false);
        log.push("note");
        assert_eq!(
            log.calls(),
            [
                PromptCall::PickOne {
                    prompt: "one".to_owned(),
                    items: vec!["a".to_owned(), "b".to_owned()],
                },
                PromptCall::PickMany {
                    prompt: "many".to_owned(),
                    items: vec!["x".to_owned(), "y".to_owned()],
                    selected: vec![true, false],
                },
                PromptCall::Input {
                    prompt: "dir".to_owned(),
                    default: "~/work".to_owned(),
                },
                PromptCall::Confirm {
                    prompt: "sure?".to_owned(),
                    default: false,
                },
                PromptCall::Prompt {
                    prompt: "note".to_owned(),
                },
            ]
        );
        assert_eq!(log.entries(), ["one", "many", "dir", "sure?", "note"]);
    }

    #[test]
    fn not_interactive_refuses_without_consuming_answers() {
        let mut script = ScriptedInteraction::new(false, vec![Answer::Confirm(true)]);
        assert!(!script.is_interactive());
        assert_matches!(
            script.confirm("sure?", true),
            Err(UiError::NotInteractive { what: "test" })
        );
        assert_matches!(
            script.pick_one("pick", &["a"]),
            Err(UiError::NotInteractive { .. })
        );
        assert_eq!(script.log().entries(), Vec::<String>::new());
    }

    #[test]
    #[should_panic(expected = "expected Answer::Confirm")]
    fn a_reply_of_the_wrong_kind_panics() {
        let mut script = ScriptedInteraction::new(true, vec![Answer::Pick(Some(0))]);
        let _ = script.confirm("sure?", true);
    }

    #[test]
    #[should_panic(expected = "no scripted answer left")]
    fn running_out_of_replies_panics() {
        let mut script = ScriptedInteraction::new(true, Vec::new());
        let _ = script.input("name", "");
    }
}
