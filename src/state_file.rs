//! Versioned JSON files in the cache directory that several `work`
//! processes update: read without a lock, written under an advisory lock
//! with an atomic replace.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::fsutil::atomic_write;
use crate::text::one_line;

/// One kind of state file.
#[derive(Debug, PartialEq, Eq)]
pub struct StateFile {
    /// File name inside the cache directory.
    pub name: &'static str,
    /// The only version this build reads or writes.
    pub version: u32,
    /// What becomes read-only when the file cannot be used, e.g. `frecency`.
    pub noun: &'static str,
    /// What replaces an unparsable file, e.g. `a new history`.
    pub fresh: &'static str,
}

/// Something wrong with a state file, reported as one warning. Paths render
/// with `{:?}` to stay on one line.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Problem {
    /// The file has a version this build does not know; it is left alone.
    #[error("{path:?} has unknown version {version}; {} is read-only for this run", .kind.noun)]
    UnknownVersion {
        /// Which file.
        kind: &'static StateFile,
        /// The file.
        path: PathBuf,
        /// The version it declares.
        version: u32,
    },
    /// The file exists but cannot be read; it is left alone.
    #[error("cannot read {path:?} ({reason}); {} is read-only for this run", .kind.noun)]
    Unreadable {
        /// Which file.
        kind: &'static StateFile,
        /// The file.
        path: PathBuf,
        /// Why (one line).
        reason: String,
    },
    /// The file is not valid JSON of its kind; it was moved aside (best
    /// effort) and the contents start empty.
    #[error(
        "{path:?} is unparsable ({reason}); moved it to {}.corrupt and started {}",
        .kind.name,
        .kind.fresh
    )]
    Corrupt {
        /// Which file.
        kind: &'static StateFile,
        /// The file.
        path: PathBuf,
        /// Why (one line).
        reason: String,
    },
}

/// Updating a state file failed. Paths render with `{:?}` to stay on one
/// line.
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    /// Writing the file failed.
    #[error("cannot save {noun} {path:?}: {source}")]
    Write {
        /// What the file holds.
        noun: &'static str,
        /// The file.
        path: PathBuf,
        /// The I/O error.
        source: io::Error,
    },
    /// Serialising failed.
    #[error("cannot encode {noun}: {source}")]
    Encode {
        /// What the file holds.
        noun: &'static str,
        /// The `serde_json` error (single-line).
        source: serde_json::Error,
    },
    /// The file cannot be changed this run.
    #[error(transparent)]
    ReadOnly(Problem),
}

/// The file as written.
#[derive(Serialize)]
struct FileOut<'a, T> {
    version: u32,
    #[serde(flatten)]
    data: &'a T,
}

/// Just the version, read first so another version is not mistaken for a
/// corrupt file. A file without one is corrupt.
#[derive(Deserialize)]
struct Peek {
    version: u32,
}

/// What reading the file found.
enum Read<T> {
    Missing,
    Loaded(T),
    UnknownVersion(u32),
    Unreadable(String),
    Corrupt(String),
}

impl StateFile {
    /// `{dir}/{name}`.
    #[must_use]
    pub fn path(&self, dir: &Path) -> PathBuf {
        dir.join(self.name)
    }

    fn read<T: DeserializeOwned>(&self, path: &Path) -> Read<T> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Read::Missing,
            Err(e) => return Read::Unreadable(one_line(&e.to_string())),
        };
        let parsed = serde_json::from_slice::<Peek>(&bytes).and_then(|Peek { version }| {
            if version == self.version {
                serde_json::from_slice::<T>(&bytes).map(Read::Loaded)
            } else {
                Ok(Read::UnknownVersion(version))
            }
        });
        parsed.unwrap_or_else(|e| Read::Corrupt(one_line(&e.to_string())))
    }

    /// The contents in `dir`, and the problem to warn about, if any; any
    /// problem comes with empty contents. An unparsable file is moved aside
    /// under the lock, only once a re-read confirms it is still unparsable.
    #[must_use]
    pub fn load<T: DeserializeOwned + Default>(&'static self, dir: &Path) -> (T, Option<Problem>) {
        let path = self.path(dir);
        let read_now = match self.read(&path) {
            Read::Corrupt(_) => {
                let _lock = self.lock(dir);
                let again = self.read(&path);
                if matches!(again, Read::Corrupt(_)) {
                    quarantine(&path);
                }
                again
            }
            other => other,
        };
        match read_now {
            Read::Missing => (T::default(), None),
            Read::Loaded(data) => (data, None),
            Read::UnknownVersion(version) => (
                T::default(),
                Some(Problem::UnknownVersion {
                    kind: self,
                    path,
                    version,
                }),
            ),
            Read::Unreadable(reason) => (
                T::default(),
                Some(Problem::Unreadable {
                    kind: self,
                    path,
                    reason,
                }),
            ),
            Read::Corrupt(reason) => (
                T::default(),
                Some(Problem::Corrupt {
                    kind: self,
                    path,
                    reason,
                }),
            ),
        }
    }

    /// Takes the exclusive advisory lock on `{dir}/{name}.lock`, released
    /// when the file is dropped. Best effort: `None` when the lock file
    /// cannot be created or the filesystem cannot lock; the write itself is
    /// still atomic.
    fn lock(&self, dir: &Path) -> Option<fs::File> {
        fs::create_dir_all(dir).ok()?;
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(format!("{}.lock", self.name)))
            .ok()?;
        file.lock().ok()?;
        Some(file)
    }

    /// Under the lock, reloads the file in `dir`, applies `f`, and writes the
    /// result atomically, so concurrent `work` processes never lose each
    /// other's changes. Returns what `f` returned, and a corrupt file that
    /// was moved aside, if any.
    ///
    /// # Errors
    ///
    /// [`StateError::ReadOnly`] for a file of unknown version or one that
    /// cannot be read (it is never overwritten and `f` does not run), or
    /// [`StateError`] when encoding or writing fails.
    pub fn update<T, R>(
        &'static self,
        dir: &Path,
        f: impl FnOnce(&mut T) -> R,
    ) -> Result<(R, Option<Problem>), StateError>
    where
        T: Serialize + DeserializeOwned + Default,
    {
        let _lock = self.lock(dir);
        let path = self.path(dir);
        let (mut data, problem) = match self.read(&path) {
            Read::Missing => (T::default(), None),
            Read::Loaded(data) => (data, None),
            Read::UnknownVersion(version) => {
                return Err(StateError::ReadOnly(Problem::UnknownVersion {
                    kind: self,
                    path,
                    version,
                }));
            }
            Read::Unreadable(reason) => {
                return Err(StateError::ReadOnly(Problem::Unreadable {
                    kind: self,
                    path,
                    reason,
                }));
            }
            Read::Corrupt(reason) => {
                quarantine(&path);
                (
                    T::default(),
                    Some(Problem::Corrupt {
                        kind: self,
                        path: path.clone(),
                        reason,
                    }),
                )
            }
        };
        let out = f(&mut data);
        let file = FileOut {
            version: self.version,
            data: &data,
        };
        let noun = self.noun;
        let mut json = serde_json::to_vec_pretty(&file)
            .map_err(|source| StateError::Encode { noun, source })?;
        json.push(b'\n');
        atomic_write(&path, &json).map_err(|source| StateError::Write { noun, path, source })?;
        Ok((out, problem))
    }
}

/// Renames a corrupt file to `{name}.corrupt`, replacing an earlier one.
/// Best effort: if the rename fails, the next write replaces the file.
fn quarantine(path: &Path) {
    let mut to = path.as_os_str().to_owned();
    to.push(".corrupt");
    let to = PathBuf::from(to);
    if fs::symlink_metadata(&to).is_ok_and(|m| m.is_dir()) {
        let _ = fs::remove_dir_all(&to);
    }
    let _ = fs::rename(path, &to);
}
