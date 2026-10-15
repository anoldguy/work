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
