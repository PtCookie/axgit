pub mod blame;
pub mod blob;
pub mod commits;
pub mod diff;
pub mod meta;
pub mod open;
pub mod readme;
pub mod refs;
pub mod resolve;
pub mod scan;
pub mod search;
pub mod tree;

use serde::Serialize;
use utoipa::ToSchema;

/// Repository list entry as defined by `GET /api/v1/repos` in docs/API.md.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RepoInfo {
    /// Repository name without the `.git` suffix.
    #[schema(example = "git-compose")]
    pub name: String,
    /// From the repo config's `[axgit]` section, falling back to `[cgit]`.
    #[schema(required = true)]
    pub section: Option<String>,
    /// From the repo config's `[axgit]` section, falling back to `[cgit]`.
    #[schema(required = true)]
    pub owner: Option<String>,
    /// From the repo config's `[axgit]` section, falling back to `[cgit]`.
    #[schema(required = true)]
    pub description: Option<String>,
    /// `None` for empty repositories (unborn HEAD).
    #[schema(required = true)]
    pub default_branch: Option<String>,
    /// Agefile (`info/web/last-modified`), falling back to the HEAD authordate.
    /// RFC 3339. `None` for empty repositories without an agefile.
    #[schema(required = true, example = "2026-07-24T13:06:00+09:00")]
    pub last_modified: Option<String>,
}

/// Repository summary as defined by `GET /api/v1/repos/{repo}` in docs/API.md.
/// Kept separate from [`RepoInfo`] — the list and summary responses are
/// distinct API contracts.
#[derive(Debug, Serialize, ToSchema)]
pub struct RepoSummary {
    #[schema(example = "git-compose")]
    pub name: String,
    #[schema(required = true)]
    pub section: Option<String>,
    #[schema(required = true)]
    pub owner: Option<String>,
    #[schema(required = true)]
    pub description: Option<String>,
    #[schema(required = true)]
    pub default_branch: Option<String>,
    #[schema(required = true, example = "2026-07-24T13:06:00+09:00")]
    pub last_modified: Option<String>,
    /// HEAD commit sha. `None` for empty repositories (unborn HEAD).
    #[schema(required = true)]
    pub head: Option<String>,
    pub branch_count: usize,
    pub tag_count: usize,
    /// `{clone_url_base}/{name}.git`; `None` when no base is configured.
    #[schema(required = true)]
    pub clone_url: Option<String>,
}
