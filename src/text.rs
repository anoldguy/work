//! Display-width helpers for terminal text (pure, std-only).
//!
//! Widths are conservative: never less than a terminal will use, so a line
//! measured, clipped or truncated with these cannot wrap.

/// Conservative terminal columns for `c`.
///
/// ASCII, the progress view's own narrow glyphs (`✓`, `✗`, braille
/// spinners) and Latin/Greek/Cyrillic letters are one column; combining
/// marks are zero; everything else (CJK, emoji, and East Asian "ambiguous"
/// symbols such as `…` and `·`) counts as two.
#[must_use]
pub(crate) const fn char_width(c: char) -> usize {
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
#[must_use]
pub(crate) fn display_width(text: &str) -> usize {
    text.chars().map(char_width).sum()
}

/// `text` padded with spaces to `width` display columns.
#[must_use]
pub(crate) fn pad_to(text: &str, width: usize) -> String {
    let fill = width.saturating_sub(display_width(text));
    format!("{text}{}", " ".repeat(fill))
}

/// The longest prefix of `text` that fits in `width` display columns, cut
/// without a marker.
#[must_use]
pub(crate) fn clip(text: &str, width: usize) -> String {
    let mut used = 0;
    text.chars()
        .take_while(|&c| {
            used += char_width(c);
            used <= width
        })
        .collect()
}

/// At most `width` display columns, ending in `…` when cut; trailing spaces
/// dropped.
#[must_use]
pub(crate) fn truncate(text: &str, width: usize) -> String {
    let text = text.trim_end();
    if display_width(text) <= width {
        return text.to_owned();
    }
    let mut cut = clip(text, width.saturating_sub(char_width('…')));
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::{char_width, clip, display_width, pad_to, truncate};

    #[test]
    fn widths_count_wide_and_combining_chars() {
        assert_eq!(char_width('a'), 1);
        assert_eq!(char_width('\u{0301}'), 0);
        assert_eq!(display_width("ab"), 2);
        assert_eq!(display_width("日本"), 4);
        assert_eq!(display_width("🚀x"), 3);
        assert_eq!(display_width("é"), 1);
    }

    #[test]
    fn pad_to_fills_display_columns() {
        assert_eq!(pad_to("日", 4), "日  ");
        assert_eq!(pad_to("long", 2), "long");
    }

    #[test]
    fn clip_never_splits_a_wide_char_over_the_edge() {
        assert_eq!(clip("日本語", 5), "日本");
        assert_eq!(clip("日本語", 6), "日本語");
        assert_eq!(clip("a🚀b", 2), "a");
        assert_eq!(clip("abc", 0), "");
    }

    #[test]
    fn truncate_marks_the_cut() {
        assert_eq!(truncate("short  ", 10), "short");
        assert_eq!(truncate("abcdefgh", 5), "abc…");
        assert_eq!(truncate("日本語です", 7), "日本…");
        assert!(display_width(&truncate("日本語です", 7)) <= 7);
    }
}
