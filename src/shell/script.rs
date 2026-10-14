//! The single-line scripts printed by `sh-*` commands (pure).

use std::path::Path;

use crate::shell::ShellError;
use crate::shell::quote::quote;

fn quoted_path(path: &Path) -> Result<String, ShellError> {
    let s = path.to_str().ok_or_else(|| ShellError::NonUtf8Path {
        path: path.to_path_buf(),
    })?;
    if s.contains(['\n', '\r']) {
        return Err(ShellError::NewlineInPath {
            path: path.to_path_buf(),
        });
    }
    // A relative path starting with '-' would be read as an option (or `-`
    // as $OLDPWD); anchor it to the current directory.
    if !path.is_absolute() && s.starts_with('-') {
        return Ok(quote(&format!("./{s}")));
    }
    Ok(quote(s))
}

/// `cd '<path>'`.
///
/// # Errors
///
/// [`ShellError::NewlineInPath`] or [`ShellError::NonUtf8Path`].
pub fn cd_script(path: &Path) -> Result<String, ShellError> {
    Ok(format!("cd {}", quoted_path(path)?))
}

/// `cd '<path>' && <editor> '<path>'`; `editor` (the caller passes
/// `EditorCmd::as_str`) is inserted verbatim as a command fragment.
///
/// # Errors
///
/// [`ShellError::NewlineInPath`] or [`ShellError::NonUtf8Path`].
pub fn on_script(path: &Path, editor: &str) -> Result<String, ShellError> {
    let q = quoted_path(path)?;
    Ok(format!("cd {q} && {editor} {q}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::assert_matches;

    #[test]
    fn cd_quotes_the_path() {
        assert_eq!(
            cd_script(Path::new("/a b/it's")).unwrap(),
            r"cd '/a b/it'\''s'"
        );
    }

    #[test]
    fn on_inserts_editor_verbatim() {
        assert_eq!(
            on_script(Path::new("/r"), "code -w").unwrap(),
            "cd '/r' && code -w '/r'"
        );
    }

    #[test]
    fn dash_leading_relative_paths_are_anchored() {
        assert_eq!(cd_script(Path::new("-")).unwrap(), "cd './-'");
        assert_eq!(
            on_script(Path::new("-x"), "vim").unwrap(),
            "cd './-x' && vim './-x'"
        );
    }

    #[test]
    fn scripts_are_single_line() {
        let s = on_script(Path::new("/a\\b'c"), "vim").unwrap();
        assert!(!s.contains('\n'));
    }

    #[test]
    fn newline_paths_are_rejected() {
        assert_matches!(
            cd_script(Path::new("/a\nb")),
            Err(ShellError::NewlineInPath { .. })
        );
        assert_matches!(
            on_script(Path::new("/a\nb"), "vim"),
            Err(ShellError::NewlineInPath { .. })
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_paths_are_rejected() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let p = Path::new(OsStr::from_bytes(b"/a\xffb"));
        assert_matches!(cd_script(p), Err(ShellError::NonUtf8Path { .. }));
        assert_matches!(on_script(p, "vim"), Err(ShellError::NonUtf8Path { .. }));
    }
}
