//! Wire shapes of the GitHub REST responses `work` reads, and their
//! validation into domain types.
//!
//! Unknown fields are ignored. Conversion is where an entry that fails
//! validation is rejected (and counted as skipped by the client).

use serde::Deserialize;

use crate::domain::{Domain, NameError, Repo};

/// `GET /user` and the `owner` of a repo.
#[derive(Debug, Deserialize)]
pub struct UserDto {
    /// The login.
    pub login: String,
}

/// One entry of `GET /user/orgs`.
#[derive(Debug, Deserialize)]
pub struct OrgDto {
    /// The org login.
    pub login: String,
}

/// One entry of a repo listing.
#[derive(Debug, Deserialize)]
pub struct RepoDto {
    /// Repository name.
    pub name: String,
    /// Owner (user or org).
    pub owner: UserDto,
    /// HTTPS clone URL.
    pub clone_url: String,
    /// Web page.
    pub html_url: String,
}

impl RepoDto {
    /// Validates the entry into a [`Repo`] on `domain`. Stricter than the
    /// domain types: API data must have an `https://` `clone_url` and
    /// `html_url`, both on `domain` (`file://`/`ssh://` clone URLs exist only
    /// for local fixtures, and an API response must not be able to point
    /// `git clone` at a local path).
    ///
    /// # Errors
    ///
    /// A [`NameError`] when any part is invalid, or either URL is not https on
    /// `domain`.
    pub fn into_repo(self, domain: &Domain) -> Result<Repo, NameError> {
        let _ = (self, domain);
        unimplemented!("W2.8")
    }
}
