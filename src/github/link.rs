//! RFC 8288 `Link` header parsing for pagination (pure).

/// The relations `work` uses from a `Link` header.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Link {
    /// Target of `rel="next"`.
    pub next: Option<String>,
    /// Target of `rel="last"`.
    pub last: Option<String>,
}

/// Parses a `Link` header value. Never fails: malformed entries are skipped,
/// and the first occurrence of each relation wins.
#[must_use]
pub fn parse_link_header(value: &str) -> Link {
    let mut link = Link::default();
    for entry in split_top_level(value, ',') {
        let Some((target, rels)) = parse_entry(entry) else {
            continue;
        };
        for rel in rels.split_ascii_whitespace() {
            if rel.eq_ignore_ascii_case("next") {
                link.next.get_or_insert_with(|| target.to_owned());
            } else if rel.eq_ignore_ascii_case("last") {
                link.last.get_or_insert_with(|| target.to_owned());
            }
        }
    }
    link
}

/// Splits on `sep` outside `<...>` and `"..."` (honouring `\` escapes inside
/// quotes). A `<` only opens a bracket at the start of a part, so a stray one
/// inside a parameter value cannot swallow later entries.
fn split_top_level(s: &str, sep: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut in_angle, mut in_quote, mut escaped) = (false, false, false);
    let mut start = 0;
    for (i, c) in s.char_indices() {
        if in_quote {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => in_quote = false,
                _ => {}
            }
        } else if in_angle {
            in_angle = c != '>';
        } else if c == '<' && s[start..i].trim().is_empty() {
            in_angle = true;
        } else if c == '"' {
            in_quote = true;
        } else if c == sep {
            parts.push(&s[start..i]);
            start = i + c.len_utf8();
        }
    }
    parts.push(&s[start..]);
    parts
}

/// Parses one `<target>; param; ...` entry into its target and `rel` value.
fn parse_entry(entry: &str) -> Option<(&str, String)> {
    let entry = entry.trim().strip_prefix('<')?;
    let (target, params) = entry.split_once('>')?;
    let target = target.trim();
    if target.is_empty() {
        return None;
    }
    let params = params.trim_start().strip_prefix(';')?;
    let rel = split_top_level(params, ';').into_iter().find_map(|p| {
        let (name, val) = p.split_once('=')?;
        name.trim()
            .eq_ignore_ascii_case("rel")
            .then(|| unquote(val.trim()))
    })?;
    Some((target, rel))
}

/// Strips surrounding quotes and resolves `\` escapes; token values pass through.
fn unquote(v: &str) -> String {
    let Some(inner) = v.strip_prefix('"') else {
        return v.to_owned();
    };
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.extend(chars.next()),
            '"' => break,
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{Link, parse_link_header};

    #[test]
    fn stray_angle_in_param_does_not_swallow_next_entry() {
        let l = parse_link_header(r#"<https://a/x>; title=<foo, <https://a/y>; rel="next""#);
        assert_eq!(l.next.as_deref(), Some("https://a/y"));
    }

    #[test]
    fn parses_next_and_last_in_either_order() {
        let a = parse_link_header(r#"<https://x/p2>; rel="next", <https://x/p9>; rel="last""#);
        let b = parse_link_header(r#"<https://x/p9>; rel="last", <https://x/p2>; rel="next""#);
        assert_eq!(a, b);
        assert_eq!(a.next.as_deref(), Some("https://x/p2"));
        assert_eq!(a.last.as_deref(), Some("https://x/p9"));
    }

    #[test]
    fn handles_multiple_rels_whitespace_and_unquoted() {
        let l = parse_link_header("  <https://x/a> ;  title=\"a,b\" ;REL = next last  ,<junk");
        assert_eq!(l.next.as_deref(), Some("https://x/a"));
        assert_eq!(l.last.as_deref(), Some("https://x/a"));
        let l = parse_link_header("<https://x/b>;rel=next");
        assert_eq!(l.next.as_deref(), Some("https://x/b"));
    }

    #[test]
    fn first_occurrence_wins_and_comma_in_target_is_kept() {
        let l = parse_link_header(r#"<https://x/a,b>; rel="next", <https://x/c>; rel="next""#);
        assert_eq!(l.next.as_deref(), Some("https://x/a,b"));
        assert_eq!(l.last, None);
    }

    #[test]
    fn ignores_other_rels_and_malformed_entries() {
        let l = parse_link_header(r#"<https://x/a>; rel="prev", garbage, <>; rel="next""#);
        assert_eq!(l, Link::default());
        assert_eq!(parse_link_header(""), Link::default());
    }

    proptest! {
        #[test]
        fn never_panics(s in any::<String>()) {
            let _ = parse_link_header(&s);
        }

        #[test]
        fn never_panics_on_header_like_input(s in r#"[<>;,="\\ a-z:/]{0,80}"#) {
            let _ = parse_link_header(&s);
        }
    }
}
