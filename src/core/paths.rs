//! Where a repository lives on disk (pure).

use std::path::{Path, PathBuf};

use crate::domain::{Domain, Slug};

/// `{work_dir}/{org}/{repo}` when the slug's domain is `default`, otherwise
/// `{work_dir}/{domain}/{org}/{repo}`.
#[must_use]
pub fn repo_path(work_dir: &Path, default: Option<&Domain>, slug: &Slug) -> PathBuf {
    let _ = (work_dir, default, slug);
    unimplemented!("W2.3")
}
