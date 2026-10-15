//! Display-width, wording and sanitising helpers for terminal text.
//!
//! Widths are conservative: never less than a terminal will use, so a line
//! measured or clipped with these cannot wrap.

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
                // An OSC without a terminator drops only the ESC and `]`, so
                // text after a stray ESC is never hidden.
                let mut rest = chars.clone();
                let terminated = loop {
                    match rest.next() {
                        Some('\x07' | '\u{9c}') => break true,
                        Some('\x1b') => {
                            if rest.peek() == Some(&'\\') {
                                rest.next();
                            }
                            break true;
                        }
                        Some(n) if n.is_control() => break false,
                        Some(_) => {}
                        None => break false,
                    }
                };
                if terminated {
                    chars = rest;
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
/// ASCII, `✓`, `✗`, braille spinners and Latin/Greek/Cyrillic letters are one column; combining
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
    use super::one_line;

    #[test]
    fn an_unterminated_escape_never_hides_the_text_after_it() {
        assert_eq!(
            one_line("unexpected '--bogus\x1b]x' found (see 'work --help')"),
            "unexpected '--bogusx' found (see 'work --help')"
        );
        assert_eq!(one_line("a\x1b]0;title\x07b"), "ab");
        assert_eq!(one_line("a\x1b]8;;url\x1b\\b"), "ab");
    }
}
