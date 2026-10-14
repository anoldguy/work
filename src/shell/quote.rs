//! One quoting scheme that POSIX sh, bash, zsh and fish all read the same
//! way (pure).

/// Quotes `s` as a single shell word: ordinary characters inside `'…'`, each
/// `'` as `\'` and each `\` as `\\` outside the quotes. `a'b\c` becomes
/// `'a'\''b'\\'c'`. The input must not contain a newline (callers check).
#[must_use]
pub fn quote(s: &str) -> String {
    let _ = s;
    unimplemented!("W1.6")
}
