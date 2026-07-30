use git2::{BranchType, ObjectType, Repository};
use serde::Serialize;
use utoipa::ToSchema;

use super::meta;

/// Branch entry of `GET /api/v1/repos/{repo}/refs` (docs/API.md).
#[derive(Debug, Serialize, ToSchema)]
pub struct BranchRef {
    #[schema(example = "main")]
    pub name: String,
    /// Commit sha of the branch tip.
    pub target: String,
    /// Authordate (RFC 3339) of the branch tip.
    #[schema(required = true)]
    pub committed_at: Option<String>,
}

/// Tag entry of `GET /api/v1/repos/{repo}/refs` (docs/API.md).
#[derive(Debug, Serialize, ToSchema)]
pub struct TagRef {
    #[schema(example = "v1.0.0")]
    pub name: String,
    /// Peeled commit sha (not the tag object), so clients can link to the commit.
    pub target: String,
    /// First line of the tag message. `None` for lightweight tags.
    #[schema(required = true)]
    pub annotation: Option<String>,
    /// Tagger date (RFC 3339). `None` for lightweight tags.
    #[schema(required = true)]
    pub tagged_at: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RefsInfo {
    /// Sorted by name ascending; empty for a repository without commits.
    pub branches: Vec<BranchRef>,
    /// Sorted by name ascending.
    pub tags: Vec<TagRef>,
}

/// Lists local branches and tags, each sorted by name.
pub fn list_refs(repo: &Repository) -> anyhow::Result<RefsInfo> {
    Ok(RefsInfo {
        branches: branches(repo)?,
        tags: tags(repo)?,
    })
}

fn branches(repo: &Repository) -> anyhow::Result<Vec<BranchRef>> {
    let mut branches = Vec::new();
    for entry in repo.branches(Some(BranchType::Local))? {
        let (branch, _) = entry?;
        let Some(name) = branch.name()?.map(str::to_owned) else {
            continue; // non-utf8 branch name
        };
        let Ok(commit) = branch.get().peel_to_commit() else {
            continue; // unresolvable tip (e.g. broken ref)
        };
        let committed_at = meta::git_time_to_zoned(commit.author().when())
            .map(|zoned| meta::format_rfc3339(&zoned));
        branches.push(BranchRef {
            name,
            target: commit.id().to_string(),
            committed_at,
        });
    }
    branches.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(branches)
}

fn tags(repo: &Repository) -> anyhow::Result<Vec<TagRef>> {
    let mut tags = Vec::new();
    for name in repo.tag_names(None)?.iter().flatten() {
        let reference = repo.find_reference(&format!("refs/tags/{name}"))?;
        // `Some` only for annotated tags; lightweight tags have no tag object.
        let tag = reference.peel_to_tag().ok();
        // Tags on non-commit objects (rare) fall back to the peeled object id.
        let target = match reference.peel_to_commit() {
            Ok(commit) => commit.id(),
            Err(_) => match reference.peel(ObjectType::Any) {
                Ok(object) => object.id(),
                Err(_) => continue,
            },
        };
        let annotation = tag
            .as_ref()
            .and_then(|tag| tag.message())
            .and_then(first_line);
        let tagged_at = tag
            .as_ref()
            .and_then(|tag| tag.tagger())
            .and_then(|tagger| meta::git_time_to_zoned(tagger.when()))
            .map(|zoned| meta::format_rfc3339(&zoned));
        tags.push(TagRef {
            name: name.to_owned(),
            target: target.to_string(),
            annotation,
            tagged_at,
        });
    }
    tags.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(tags)
}

fn first_line(message: &str) -> Option<String> {
    let line = message.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_owned())
}
