//! Display-width, wording and sanitising helpers for terminal text (pure,
//! std-only).
//!
//! Widths are conservative: never less than a terminal will use, so a line
//! measured or clipped with these cannot wrap.
//!
//! [`is_invisible_format`] is the crate's one list of invisible Unicode
//! format characters, and [`one_line`] the one sanitiser; every check and
//! sanitiser that keeps text on one visible line goes through them.

/// Bidi controls (including the Arabic letter mark), zero-width characters,
/// the soft hyphen, the byte-order mark, and the line and paragraph
/// separators (U+2028, U+2029): characters that are invisible, or break a
/// line, without being `char::is_control`.
///
/// ```
/// use work::text::is_invisible_format;
///
/// assert!(is_invisible_format('\u{202E}')); // right-to-left override
/// assert!(is_invisible_format('\u{2028}')); // line separator
/// assert!(!is_invisible_format('a'));
/// ```
#[must_use]
pub const fn is_invisible_format(c: char) -> bool {
    matches!(
        c,
        '\u{AD}'
            | '\u{061C}'
            | '\u{200B}'..='\u{200F}'
            | '\u{2028}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
    )
}

/// True for a character that must not reach a one-line message as is: a
/// control character or an [invisible format character](is_invisible_format).
///
/// ```
/// use work::text::is_unsafe;
///
/// assert!(is_unsafe('\n'));
/// assert!(is_unsafe('\u{2029}'));
/// assert!(!is_unsafe(' '));
/// ```
#[must_use]
pub fn is_unsafe(c: char) -> bool {
    c.is_control() || is_invisible_format(c)
}

/// `text` made safe for one visible line: ANSI escape sequences are
/// removed, line breaks and other whitespace collapse to single spaces,
/// the remaining [unsafe](is_unsafe) characters (controls, bidi and
/// zero-width characters) are dropped, and the ends are trimmed.
///
/// ```
/// use work::text::one_line;
///
/// assert_eq!(one_line("  bad\n\tvalue\u{2029} here "), "bad value here");
/// assert_eq!(one_line("\x1b[31mevil\u{202E}txt\0.exe\x1b[0m"), "eviltxt.exe");
/// ```
#[must_use]
pub fn one_line(text: &str) -> String {
    let visible: String = strip_escapes(text)
        .chars()
        .filter_map(|c| match c {
            c if c.is_whitespace() => Some(' '),
            c if is_unsafe(c) => None,
            c => Some(c),
        })
        .collect();
    visible.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Removes ANSI escape sequences: CSI (`ESC [ ... final`), OSC
/// (`ESC ] ... BEL|ST`) and two-byte escapes.
fn strip_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for n in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&n) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(n) = chars.next() {
                    if n == '\x07' || n == '\u{9c}' {
                        break;
                    }
                    if n == '\x1b' {
                        if chars.peek() == Some(&'\\') {
                            chars.next();
                        }
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// `one` when `n` is 1, otherwise `many`: `plural(2, "repo", "repos")`.
#[must_use]
pub(crate) fn plural<'w, N: Copy + PartialEq + From<u8>>(
    n: N,
    one: &'w str,
    many: &'w str,
) -> &'w str {
    if n == N::from(1) { one } else { many }
}

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
#[cfg(test)]
#[must_use]
pub(crate) fn display_width(text: &str) -> usize {
    text.chars().map(char_width).sum()
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

#[cfg(test)]
mod tests {
    use super::{
        char_width, clip, display_width, is_invisible_format, is_unsafe, one_line, plural,
    };

    /// Every bidi control: embeddings/overrides, isolates, marks.
    const BIDI: [char; 12] = [
        '\u{061C}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}',
        '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
    ];

    #[test]
    fn separators_bidi_controls_and_zero_widths_are_invisible() {
        for c in BIDI
            .into_iter()
            .chain(['\u{2028}', '\u{2029}', '\u{200B}', '\u{FEFF}', '\u{AD}'])
        {
            assert!(is_invisible_format(c), "{c:?}");
            assert!(is_unsafe(c), "{c:?}");
        }
        for c in ['a', ' ', 'é', '日', '…'] {
            assert!(!is_unsafe(c), "{c:?}");
        }
        assert!(is_unsafe('\u{1b}'));
    }

    #[test]
    fn one_line_breaks_no_line_and_hides_no_text() {
        assert_eq!(one_line("a\u{2028}b\u{2029}c"), "a b c");
        assert_eq!(one_line("a\r\n\tb\0c"), "a bc");
        for c in BIDI.into_iter().chain(['\u{200B}', '\u{FEFF}']) {
            assert_eq!(one_line(&format!("x{c}y")), "xy", "{c:?}");
        }
        assert_eq!(one_line(" a \u{2028}\u{2029} b\n"), "a b");
    }

    #[test]
    fn one_line_strips_whole_ansi_sequences() {
        assert_eq!(one_line("\x1b[1;31mboom\x1b[0m"), "boom");
        assert_eq!(one_line("\x1b]0;title\x07ok"), "ok");
        assert_eq!(one_line("\x1b]8;;http://x\x1b\\link\x1b]8;;\x1b\\"), "link");
    }

    #[test]
    fn plural_picks_the_singular_only_for_one() {
        assert_eq!(plural(1_usize, "repo", "repos"), "repo");
        assert_eq!(plural(0_u32, "repo", "repos"), "repos");
        assert_eq!(plural(2_u64, "repo", "repos"), "repos");
    }

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
    fn clip_never_splits_a_wide_char_over_the_edge() {
        assert_eq!(clip("日本語", 5), "日本");
        assert_eq!(clip("日本語", 6), "日本語");
        assert_eq!(clip("a🚀b", 2), "a");
        assert_eq!(clip("abc", 0), "");
    }
}
