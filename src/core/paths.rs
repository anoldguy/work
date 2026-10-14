//! Where a repository lives on disk (pure).

use std::path::{Path, PathBuf};

use crate::domain::{Domain, Slug};

/// `{work_dir}/{org}/{repo}` when the slug's domain is `default`, otherwise
/// `{work_dir}/{domain}/{org}/{repo}`.
///
/// With no default domain configured, the domain segment is always included.
///
/// The org and repo keep the API's casing, so on a case-insensitive filesystem
/// (macOS) `Acme/foo` and `acme/foo` name the same directory.
#[must_use]
pub fn repo_path(work_dir: &Path, default: Option<&Domain>, slug: &Slug) -> PathBuf {
    let mut path = work_dir.to_path_buf();
    if default != Some(slug.domain()) {
        path.push(slug.domain().as_str());
    }
    path.push(slug.org().as_str());
    path.push(slug.name().as_str());
    path
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::repo_path;
    use crate::domain::{Domain, Slug};

    fn slug(raw: &str) -> Slug {
        raw.parse().expect("test slug is valid")
    }

    fn domain(raw: &str) -> Domain {
        raw.parse().expect("test domain is valid")
    }

    #[test]
    fn default_domain_repo_omits_the_domain_segment() {
        let work = Path::new("/work");
        let github = domain("github.com");
        let path = repo_path(work, Some(&github), &slug("github.com/acme/widgets"));
        assert_eq!(path, Path::new("/work/acme/widgets"));
    }

    #[test]
    fn non_default_domain_repo_includes_the_domain_segment() {
        let work = Path::new("/work");
        let github = domain("github.com");
        let path = repo_path(
            work,
            Some(&github),
            &slug("github.enterprise.com/acme/widgets"),
        );
        assert_eq!(path, Path::new("/work/github.enterprise.com/acme/widgets"));
    }

    #[test]
    fn no_default_configured_always_includes_the_domain() {
        let work = Path::new("/work");
        let path = repo_path(work, None, &slug("github.com/acme/widgets"));
        assert_eq!(path, Path::new("/work/github.com/acme/widgets"));
    }
}
