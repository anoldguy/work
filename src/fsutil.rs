//! Small filesystem helpers shared by the stores.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Writes `bytes` to `path` atomically.
///
/// A symlink at `path` is resolved first, so the write goes through to its
/// target instead of replacing the link. Parent directories are created. The
/// data goes to an exclusive temporary file (`<name>.tmp-<pid>-<n>`) beside
/// the target, is fsynced, renamed into place, and the directory is fsynced.
/// An existing file's permissions are kept. On failure the temporary file is
/// removed. Concurrent writers: the last rename wins.
///
/// # Errors
///
/// Any I/O error from creating directories, writing, renaming, or syncing.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    static SEQ: AtomicU64 = AtomicU64::new(0);

    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let dir = match target.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    fs::create_dir_all(dir)?;
    let mut name = target
        .file_name()
        .map_or_else(|| OsString::from("file"), OsString::from);
    name.push(format!(
        ".tmp-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp = dir.join(name);

    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(bytes)?;
        if let Ok(meta) = fs::metadata(&target) {
            file.set_permissions(meta.permissions())?;
        }
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, &target)
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    // Make the rename durable. Some platforms cannot open or sync a
    // directory; the data is already in place, so that is not an error.
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_parents_and_writes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a/b/f.txt");
        atomic_write(&p, b"hi").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"hi");
    }

    #[test]
    fn replaces_existing_and_leaves_no_temp() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f");
        atomic_write(&p, b"one").unwrap();
        atomic_write(&p, b"two").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"two");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn writes_through_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real/f");
        fs::create_dir(dir.path().join("real")).unwrap();
        fs::write(&real, "old").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        atomic_write(&link, b"new").unwrap();
        assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(fs::read(&real).unwrap(), b"new");
    }

    #[cfg(unix)]
    #[test]
    fn keeps_existing_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f");
        fs::write(&p, "x").unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
        atomic_write(&p, b"y").unwrap();
        assert_eq!(
            fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn fails_when_parent_is_a_file_and_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("f");
        fs::write(&f, "x").unwrap();
        assert!(atomic_write(&f.join("child"), b"y").is_err());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
