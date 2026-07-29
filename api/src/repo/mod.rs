pub mod commits;
pub mod meta;
pub mod open;
pub mod refs;
pub mod resolve;
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

/// Repository summary as defined by `GET /api/v1/repos/{repo}` in docs/API.md.
/// Kept separate from [`RepoInfo`] — the list and summary responses are
/// distinct API contracts.
#[derive(Debug, Serialize)]
pub struct RepoSummary {
    pub name: String,
    pub section: Option<String>,
    pub owner: Option<String>,
    pub description: Option<String>,
    pub default_branch: Option<String>,
    pub last_modified: Option<String>,
    /// HEAD commit sha. `None` for empty repositories (unborn HEAD).
    pub head: Option<String>,
    pub branch_count: usize,
    pub tag_count: usize,
    /// `{clone_url_base}/{name}.git`; `None` when no base is configured.
    pub clone_url: Option<String>,
}
