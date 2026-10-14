//! [`TtyInteraction`]: pickers and prompts on the real terminal, and
//! `Ui::interact`.

use std::io::{self, IsTerminal};

use crate::ui::{Interaction, Ui, UiError};

/// The real [`Interaction`]: renders `picker::PickerState` and the prompts
/// on its own stderr handle inside a `TerminalGuard` (`Screen::Inline`).
#[derive(Debug)]
pub struct TtyInteraction {
    err: io::Stderr,
    interactive: bool,
}

impl TtyInteraction {
    /// Checks whether stdin and stderr are terminals.
    #[must_use]
    pub fn new() -> Self {
        let err = io::stderr();
        let interactive = io::stdin().is_terminal() && err.is_terminal();
        Self { err, interactive }
    }
}

impl Default for TtyInteraction {
    fn default() -> Self {
        Self::new()
    }
}

impl Interaction for TtyInteraction {
    fn is_interactive(&self) -> bool {
        self.interactive
    }

    fn pick_one(&mut self, prompt: &str, items: &[&str]) -> Result<Option<usize>, UiError> {
        let _ = (&mut self.err, prompt, items);
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

impl Ui {
    /// The pickers and prompts. Borrowing the Ui keeps them from
    /// interleaving with any other output.
    pub fn interact(&mut self) -> &mut dyn Interaction {
        &mut *self.interact
    }
}
