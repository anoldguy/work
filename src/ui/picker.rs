//! The picker as a pure reducer: keys in, steps and frame lines out.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// How far `PageUp`/`PageDown` move the cursor.
const PAGE: usize = 10;

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
        if event.kind == KeyEventKind::Release {
            return None;
        }
        let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
        let alt = event.modifiers.contains(KeyModifiers::ALT);
        match event.code {
            KeyCode::Char(c) if ctrl => match c.to_ascii_lowercase() {
                'c' => Some(Self::CtrlC),
                'p' => Some(Self::Up),
                'n' => Some(Self::Down),
                _ => None,
            },
            KeyCode::Char(c) if !alt && !c.is_control() => Some(Self::Char(c)),
            KeyCode::Backspace => Some(Self::Backspace),
            KeyCode::Up => Some(Self::Up),
            KeyCode::Down => Some(Self::Down),
            KeyCode::PageUp => Some(Self::PageUp),
            KeyCode::PageDown => Some(Self::PageDown),
            KeyCode::Tab => Some(Self::Toggle),
            KeyCode::Enter => Some(Self::Enter),
            KeyCode::Esc => Some(Self::Esc),
            _ => None,
        }
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
/// rank order. When the filter changes the cursor stays on its item if that
/// item still matches, and is clamped otherwise. Multi-select
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
    /// `selected` (same length) makes it multi-select. A `selected` of the
    /// wrong length is a caller bug: it asserts in debug builds and is padded
    /// with `false` or truncated in release builds.
    #[must_use]
    pub fn new(items: &'a [&'a str], selected: Option<Vec<bool>>) -> Self {
        if let Some(s) = &selected {
            debug_assert_eq!(s.len(), items.len(), "one selection flag per item");
        }
        let selected = selected.map(|mut s| {
            s.resize(items.len(), false);
            s
        });
        Self {
            items,
            filter: String::new(),
            visible: (0..items.len()).collect(),
            cursor: 0,
            selected,
        }
    }

    /// Applies one key.
    pub fn apply(&mut self, key: PickerKey) -> PickerStep {
        match key {
            PickerKey::Char(c) => {
                self.filter.push(c);
                self.refilter();
            }
            PickerKey::Backspace => {
                if self.filter.pop().is_some() {
                    self.refilter();
                }
            }
            PickerKey::Up => self.cursor = self.cursor.saturating_sub(1),
            PickerKey::Down => self.move_down(1),
            PickerKey::PageUp => self.cursor = self.cursor.saturating_sub(PAGE),
            PickerKey::PageDown => self.move_down(PAGE),
            PickerKey::Toggle => {
                if let Some(selected) = &mut self.selected
                    && let Some(&index) = self.visible.get(self.cursor)
                {
                    selected[index] = !selected[index];
                }
            }
            PickerKey::Enter => {
                let Some(&index) = self.visible.get(self.cursor) else {
                    return PickerStep::Continue;
                };
                return PickerStep::Chosen(match &self.selected {
                    None => vec![index],
                    Some(selected) => selected
                        .iter()
                        .enumerate()
                        .filter_map(|(i, &on)| on.then_some(i))
                        .collect(),
                });
            }
            PickerKey::Esc => return PickerStep::Cancelled,
            PickerKey::CtrlC => return PickerStep::Interrupted,
        }
        PickerStep::Continue
    }

    /// The frame for a `w`×`h` area: the filter line, then the visible window
    /// of items with the cursor marked. Every line is at most `w` characters
    /// and there are at most `h` lines. The caller draws its prompt text
    /// above the frame.
    #[must_use]
    pub fn lines(&self, w: u16, h: u16) -> Vec<String> {
        let (w, h) = (usize::from(w), usize::from(h));
        if w == 0 || h == 0 {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(h);
        out.push(fit(
            &format!(
                "> {}  ({}/{})",
                self.filter,
                self.visible.len(),
                self.items.len()
            ),
            w,
        ));
        let rows = h - 1;
        if rows == 0 {
            return out;
        }
        if self.visible.is_empty() {
            out.push(fit("  (no matches)", w));
            return out;
        }
        let start = self.cursor.saturating_sub(rows - 1);
        out.extend(
            self.visible
                .iter()
                .enumerate()
                .skip(start)
                .take(rows)
                .map(|(pos, &index)| {
                    let marker = if pos == self.cursor { '>' } else { ' ' };
                    let check = match &self.selected {
                        None => "",
                        Some(selected) if selected[index] => "[x] ",
                        Some(_) => "[ ] ",
                    };
                    fit(&format!("{marker} {check}{}", self.items[index]), w)
                }),
        );
        out
    }

    fn move_down(&mut self, by: usize) {
        let last = self.visible.len().saturating_sub(1);
        self.cursor = self.cursor.saturating_add(by).min(last);
    }

    fn refilter(&mut self) {
        let highlighted = self.visible.get(self.cursor).copied();
        let needle = self.filter.to_lowercase();
        self.visible = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.to_lowercase().contains(&needle))
            .map(|(i, _)| i)
            .collect();
        self.cursor = highlighted
            .and_then(|item| self.visible.iter().position(|&i| i == item))
            .unwrap_or_else(|| self.cursor.min(self.visible.len().saturating_sub(1)));
    }
}

/// The first `w` characters of `s`.
fn fit(s: &str, w: usize) -> String {
    s.chars().take(w).collect()
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    use super::{PickerKey, PickerState, PickerStep};

    const ITEMS: &[&str] = &[
        "acme/widgets",
        "acme/Gadgets",
        "other/widget-tools",
        "acme/docs",
        "jk/keys",
    ];

    fn typed(state: &mut PickerState<'_>, text: &str) {
        for c in text.chars() {
            assert_eq!(state.apply(PickerKey::Char(c)), PickerStep::Continue);
        }
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn typing_narrows_case_insensitively_in_rank_order() {
        let mut state = PickerState::new(ITEMS, None);
        typed(&mut state, "WIDGET");
        assert_eq!(state.visible, vec![0, 2]);
        state.apply(PickerKey::Down);
        assert_eq!(state.apply(PickerKey::Enter), PickerStep::Chosen(vec![2]));
    }

    #[test]
    fn backspace_widens_the_filter_again() {
        let mut state = PickerState::new(ITEMS, None);
        typed(&mut state, "gadx");
        assert_eq!(state.visible, Vec::<usize>::new());
        state.apply(PickerKey::Backspace);
        assert_eq!(state.visible, vec![1]);
        state.apply(PickerKey::Backspace);
        state.apply(PickerKey::Backspace);
        state.apply(PickerKey::Backspace);
        assert_eq!(state.filter, "");
        assert_eq!(state.visible.len(), ITEMS.len());
        assert_eq!(state.apply(PickerKey::Backspace), PickerStep::Continue);
    }

    #[test]
    fn arrows_move_and_stop_at_the_ends() {
        let mut state = PickerState::new(ITEMS, None);
        state.apply(PickerKey::Up);
        assert_eq!(state.cursor, 0);
        for _ in 0..10 {
            state.apply(PickerKey::Down);
        }
        assert_eq!(state.cursor, ITEMS.len() - 1);
        state.apply(PickerKey::PageUp);
        assert_eq!(state.cursor, 0);
        state.apply(PickerKey::PageDown);
        assert_eq!(state.cursor, ITEMS.len() - 1);
    }

    #[test]
    fn j_and_k_are_filter_text_not_navigation() {
        let mut state = PickerState::new(ITEMS, None);
        typed(&mut state, "jk");
        assert_eq!(state.cursor, 0);
        assert_eq!(state.visible, vec![4]);
        assert_eq!(state.apply(PickerKey::Enter), PickerStep::Chosen(vec![4]));
    }

    #[test]
    fn the_cursor_is_clamped_when_the_filter_shrinks_the_list() {
        let mut state = PickerState::new(ITEMS, None);
        for _ in 0..4 {
            state.apply(PickerKey::Down);
        }
        typed(&mut state, "acme");
        assert_eq!(state.visible, vec![0, 1, 3]);
        assert_eq!(state.cursor, 2);
        typed(&mut state, "zzz");
        assert_eq!(state.cursor, 0);
    }

    #[test]
    fn the_cursor_stays_on_its_item_when_it_still_matches() {
        let items = &["b", "a1", "a2", "c"];
        let mut state = PickerState::new(items, None);
        state.apply(PickerKey::Down);
        typed(&mut state, "a");
        assert_eq!(state.cursor, 0);
        assert_eq!(state.apply(PickerKey::Enter), PickerStep::Chosen(vec![1]));
        state.apply(PickerKey::Backspace);
        assert_eq!(state.apply(PickerKey::Enter), PickerStep::Chosen(vec![1]));
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "one selection flag per item")]
    fn a_wrong_length_selection_is_a_caller_bug() {
        let _ = PickerState::new(ITEMS, Some(vec![true]));
    }

    #[test]
    fn enter_on_an_empty_list_is_a_no_op() {
        let mut state = PickerState::new(ITEMS, None);
        typed(&mut state, "nothing matches");
        assert_eq!(state.apply(PickerKey::Enter), PickerStep::Continue);
        let mut empty = PickerState::new(&[], None);
        assert_eq!(empty.apply(PickerKey::Enter), PickerStep::Continue);
    }

    #[test]
    fn esc_cancels_and_ctrl_c_interrupts() {
        let mut state = PickerState::new(ITEMS, None);
        assert_matches!(state.apply(PickerKey::Esc), PickerStep::Cancelled);
        assert_matches!(state.apply(PickerKey::CtrlC), PickerStep::Interrupted);
    }

    #[test]
    fn tab_is_ignored_in_single_select() {
        let mut state = PickerState::new(ITEMS, None);
        assert_eq!(state.apply(PickerKey::Toggle), PickerStep::Continue);
        assert_eq!(state.apply(PickerKey::Enter), PickerStep::Chosen(vec![0]));
    }

    #[test]
    fn multi_select_toggles_survive_filter_changes() {
        let mut state = PickerState::new(ITEMS, Some(vec![false, false, false, true, false]));
        typed(&mut state, "widget");
        state.apply(PickerKey::Down);
        state.apply(PickerKey::Toggle);
        for _ in 0.."widget".len() {
            state.apply(PickerKey::Backspace);
        }
        typed(&mut state, "docs");
        state.apply(PickerKey::Toggle);
        typed(&mut state, "zzz");
        assert_eq!(state.apply(PickerKey::Enter), PickerStep::Continue);
        for _ in 0..3 {
            state.apply(PickerKey::Backspace);
        }
        state.apply(PickerKey::Up);
        state.apply(PickerKey::Toggle);
        state.apply(PickerKey::Toggle);
        assert_eq!(state.apply(PickerKey::Enter), PickerStep::Chosen(vec![2]));
    }

    #[test]
    fn multi_select_may_choose_nothing() {
        let mut state = PickerState::new(ITEMS, Some(vec![true, false, false, false, false]));
        state.apply(PickerKey::Toggle);
        assert_eq!(state.apply(PickerKey::Enter), PickerStep::Chosen(vec![]));
    }

    #[test]
    fn keys_decode_per_the_spec() {
        let none = KeyModifiers::NONE;
        let ctrl = KeyModifiers::CONTROL;
        let cases = [
            (key(KeyCode::Char('j'), none), Some(PickerKey::Char('j'))),
            (
                key(KeyCode::Char('Q'), KeyModifiers::SHIFT),
                Some(PickerKey::Char('Q')),
            ),
            (key(KeyCode::Char('p'), ctrl), Some(PickerKey::Up)),
            (key(KeyCode::Char('n'), ctrl), Some(PickerKey::Down)),
            (key(KeyCode::Char('c'), ctrl), Some(PickerKey::CtrlC)),
            (key(KeyCode::Char('x'), ctrl), None),
            (key(KeyCode::Char('x'), KeyModifiers::ALT), None),
            (key(KeyCode::Up, none), Some(PickerKey::Up)),
            (key(KeyCode::Down, none), Some(PickerKey::Down)),
            (key(KeyCode::PageUp, none), Some(PickerKey::PageUp)),
            (key(KeyCode::PageDown, none), Some(PickerKey::PageDown)),
            (key(KeyCode::Tab, none), Some(PickerKey::Toggle)),
            (key(KeyCode::Backspace, none), Some(PickerKey::Backspace)),
            (key(KeyCode::Enter, none), Some(PickerKey::Enter)),
            (key(KeyCode::Esc, none), Some(PickerKey::Esc)),
            (key(KeyCode::F(1), none), None),
        ];
        for (event, expected) in cases {
            assert_eq!(PickerKey::from_event(&event), expected, "{event:?}");
        }
        let release = KeyEvent::new_with_kind(KeyCode::Enter, none, KeyEventKind::Release);
        assert_eq!(PickerKey::from_event(&release), None);
    }

    #[test]
    fn frame_with_a_filter_applied() {
        let mut state = PickerState::new(ITEMS, None);
        typed(&mut state, "ACME");
        state.apply(PickerKey::Down);
        insta::assert_snapshot!(state.lines(40, 6).join("\n"), @r"
        > ACME  (3/5)
          acme/widgets
        > acme/Gadgets
          acme/docs
        ");
    }

    #[test]
    fn frame_scrolls_to_keep_the_cursor_visible_and_truncates() {
        let mut state = PickerState::new(ITEMS, None);
        typed(&mut state, "s");
        for _ in 0..3 {
            state.apply(PickerKey::Down);
        }
        insta::assert_snapshot!(state.lines(12, 3).join("\n"), @r"
        > s  (5/5)
          other/widg
        > acme/docs
        ");
    }

    #[test]
    fn frame_for_multi_select_marks_selections() {
        let mut state = PickerState::new(ITEMS, Some(vec![true, false, true, false, false]));
        typed(&mut state, "widget");
        insta::assert_snapshot!(state.lines(40, 10).join("\n"), @r"
        > widget  (2/5)
        > [x] acme/widgets
          [x] other/widget-tools
        ");
    }

    #[test]
    fn frame_with_no_matches_says_so() {
        let mut state = PickerState::new(ITEMS, None);
        typed(&mut state, "zz");
        insta::assert_snapshot!(state.lines(40, 10).join("\n"), @r"
        > zz  (0/5)
          (no matches)
        ");
    }

    #[test]
    fn frame_of_zero_size_is_empty() {
        let state = PickerState::new(ITEMS, None);
        assert_eq!(state.lines(0, 5), Vec::<String>::new());
        assert_eq!(state.lines(5, 0), Vec::<String>::new());
        assert_eq!(state.lines(80, 1).len(), 1);
    }
}
