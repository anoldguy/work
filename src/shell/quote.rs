//! One quoting scheme that POSIX sh, bash, zsh and fish all read the same
//! way (pure).

/// Quotes `s` as a single shell word: ordinary characters inside `'…'`, each
/// `'` as `\'` and each `\` as `\\` outside the quotes. `a'b\c` becomes
/// `'a'\''b'\\'c'`. The input must not contain a newline (callers check).
///
/// ```
/// use work::shell::quote;
///
/// assert_eq!(quote("plain"), "'plain'");
/// assert_eq!(quote("it's"), r"'it'\''s'");
/// assert_eq!(quote(r"a'b\c"), r"'a'\''b'\\'c'");
/// assert_eq!(quote("$HOME; rm -rf /"), "'$HOME; rm -rf /'");
/// ```
#[must_use]
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        match c {
            '\'' => out.push_str(r"'\''"),
            '\\' => out.push_str(r"'\\'"),
            _ => out.push(c),
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::process::{Command, Stdio};

    #[test]
    fn quotes_the_documented_example() {
        assert_eq!(quote(r"a'b\c"), r"'a'\''b'\\'c'");
        assert_eq!(quote(""), "''");
        assert_eq!(quote("plain"), "'plain'");
    }

    fn on_path(bin: &str) -> bool {
        Command::new(bin)
            .args(["-c", "true"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
    }

    fn round_trip(shell: &str, s: &str) -> bool {
        let script = format!("printf %s {}", quote(s));
        let out = Command::new(shell)
            .args(["-c", &script])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        out.status.success() && out.stdout == s.as_bytes()
    }

    #[test]
    fn round_trips_samples_through_each_available_shell() {
        let samples = [
            "",
            "a b",
            "it's",
            r"back\slash",
            r"\'",
            r"''\\''",
            "$HOME `x` $(y) *? [a] ~ #! &;|<>",
            "-n",
            "ünï cødé 日本",
            "{a,b} \"q\"",
        ];
        for shell in ["sh", "bash", "zsh", "fish"] {
            if !on_path(shell) {
                eprintln!("note: {shell} not on PATH; skipping quote round trip");
                continue;
            }
            for s in samples {
                assert!(round_trip(shell, s), "{shell} failed on {s:?}");
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        #[test]
        fn sh_round_trips_arbitrary_paths(s in "[^\n\0]{0,40}") {
            prop_assert!(round_trip("sh", &s));
        }

        #[test]
        fn sh_round_trips_quote_heavy_paths(s in r#"[a '\\$"]{0,30}"#) {
            prop_assert!(round_trip("sh", &s));
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(8))]

        #[test]
        fn other_shells_round_trip_arbitrary_paths(s in "[^\n\0]{0,30}") {
            for shell in ["bash", "zsh", "fish"] {
                if on_path(shell) {
                    prop_assert!(round_trip(shell, &s), "{} failed", shell);
                }
            }
        }
    }
}
