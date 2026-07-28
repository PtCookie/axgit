pub mod meta;
pub mod scan;

use serde::Serialize;

/// Repository list entry as defined by `GET /api/v1/repos` in docs/API.md.
#[derive(Debug, Clone, Serialize)]
pub struct RepoInfo {
    pub name: String,
    pub section: Option<String>,
    pub owner: Option<String>,
    pub description: Option<String>,
    /// `None` for empty repositories (unborn HEAD).
    pub default_branch: Option<String>,
    /// RFC 3339. `None` for empty repositories without an agefile.
    pub last_modified: Option<String>,
}
