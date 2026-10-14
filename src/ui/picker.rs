//! The picker as a pure reducer: keys in, steps and frame lines out.

use crossterm::event::KeyEvent;

/// A picker key after decoding (PLAN §10 Q17). Letters are never navigation;
/// they extend the filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerKey {
    /// A printable character for the filter.
    Char(char),
    /// Delete the last filter character.
    Backspace,
    /// Up or Ctrl-p.
    Up,
    /// Down or Ctrl-n.
    Down,
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
    /// Tab: toggle the highlighted entry (multi-select only).
    Toggle,
    /// Choose.
    Enter,
    /// Cancel.
    Esc,
    /// Abort the command (exit 130).
    CtrlC,
}

impl PickerKey {
    /// Decodes a terminal key event; `None` for keys the picker ignores.
    #[must_use]
    pub fn from_event(event: &KeyEvent) -> Option<Self> {
        let _ = event;
        unimplemented!("W1.8")
    }
}

/// What the caller does after a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerStep {
    /// Keep reading keys and redraw.
    Continue,
    /// Done: the chosen item indices (one for single-select; the toggled set,
    /// possibly empty, for multi-select).
    Chosen(Vec<usize>),
    /// Esc.
    Cancelled,
    /// Ctrl-C.
    Interrupted,
}

/// Picker state: items in rank order, the filter, and the cursor.
///
/// Typing narrows to items containing the filter (case-insensitive), keeping
/// rank order. The cursor is clamped when the filter changes. Multi-select
/// selections are by item index, so they survive filter changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerState<'a> {
    items: &'a [&'a str],
    filter: String,
    visible: Vec<usize>,
    cursor: usize,
    selected: Option<Vec<bool>>,
}

impl<'a> PickerState<'a> {
    /// A picker over `items` (borrowed from the `Interaction` call);
    /// `selected` (same length) makes it multi-select.
    #[must_use]
    pub fn new(items: &'a [&'a str], selected: Option<Vec<bool>>) -> Self {
        let _ = (items, selected);
        unimplemented!("W1.8")
    }

    /// Applies one key.
    pub fn apply(&mut self, key: PickerKey) -> PickerStep {
        let _ = (
            self.items,
            &mut self.filter,
            &mut self.visible,
            &mut self.cursor,
            &mut self.selected,
            key,
        );
        unimplemented!("W1.8")
    }

    /// The frame for a `w`×`h` area: prompt and filter line, then the visible
    /// window of items with the cursor marked.
    #[must_use]
    pub fn lines(&self, w: u16, h: u16) -> Vec<String> {
        let _ = (self, w, h);
        unimplemented!("W1.8")
    }
}
