use git2::{BranchType, ReferenceType, Repository};
use serde::Serialize;
use utoipa::ToSchema;

use super::meta;
use super::tag::{self, TagObject};
use crate::error::ApiError;

/// Branch entry of `GET /api/v1/repos/{repo}/refs` (api/README.md).
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

/// Tag entry of `GET /api/v1/repos/{repo}/refs` (api/README.md).
#[derive(Debug, Serialize, ToSchema)]
pub struct TagRef {
    #[schema(example = "v1.0.0")]
    pub name: String,
    /// The tag's one-level dereference — same meaning as `GET /tags/{name}`'s
    /// `object` (docs/DECISIONS.md #51/#53).
    pub object: TagObject,
    /// Fully peeled commit sha, same meaning as `GET /tags/{name}`'s
    /// `target`. `None` when the tag chain never reaches a commit (a tag on
    /// a tree or blob) — also exactly when an archive download and a
    /// Compare link are unavailable for this tag.
    #[schema(required = true)]
    pub target: Option<String>,
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
    /// Remote-tracking branches (`refs/remotes/*`), sorted by name ascending.
    /// Empty on essentially every repository axgit serves today: neither
    /// axgit nor the git-compose stack that populates `/srv/git` ever runs
    /// `git remote add`/`git fetch` against a served bare repository (pushes
    /// arrive via SSH only, CLAUDE.md's read-only invariant) — this field
    /// exists for the rare case of a repository someone configured that way
    /// by hand, not for anything axgit itself produces.
    pub remote_branches: Vec<BranchRef>,
    /// Sorted by name ascending.
    pub tags: Vec<TagRef>,
}

/// Lists local branches, remote-tracking branches, and tags, each sorted by name.
pub fn list_refs(repo: &Repository) -> anyhow::Result<RefsInfo> {
    Ok(RefsInfo {
        branches: branches_of_kind(repo, BranchType::Local)?,
        remote_branches: branches_of_kind(repo, BranchType::Remote)?,
        tags: tags(repo)?,
    })
}

/// Shared by `branches`/`remote_branches` above (`repo.branches(Some(kind))`
/// only differs in `kind`, and the tuple's second element — the kind itself
/// — was previously discarded with `_`).
fn branches_of_kind(repo: &Repository, kind: BranchType) -> anyhow::Result<Vec<BranchRef>> {
    let mut branches = Vec::new();
    for entry in repo.branches(Some(kind))? {
        let (branch, _) = entry?;
        // A remote's own `HEAD` (e.g. `refs/remotes/origin/HEAD`) is a
        // symbolic ref aliasing whichever branch that remote's default is —
        // libgit2's remote-branch iteration selects purely on the
        // `refs/remotes/` prefix and doesn't exclude it, and it resolves to
        // a commit just fine, so the usual "unresolvable tip" guard below
        // wouldn't catch it either. Skipping it here avoids listing what is
        // really just an alias for another row as its own branch.
        if branch.get().kind() != Some(ReferenceType::Direct) {
            continue;
        }
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
    // git2 0.21's `StringArray` iterator yields `Result<Option<&str>, Error>`
    // — `Err` for a non-utf8 name, `None` for an absent slot. Both are skipped,
    // exactly as flattening the plain `Option` did before.
    for name in repo
        .tag_names(None)?
        .iter()
        .filter_map(|name| name.ok().flatten())
    {
        let reference = repo.find_reference(&format!("refs/tags/{name}"))?;
        // `Some` only for annotated tags; lightweight tags have no tag object.
        let tag = reference.peel_to_tag().ok();
        // `tag::dereference` is the same one-level-dereference-plus-peeled-
        // commit logic `GET /tags/{name}` uses — a tag on a non-commit object
        // now reports its real kind via `object.type` instead of `target`
        // silently holding a non-commit oid (the bug docs/DECISIONS.md #53
        // fixes). A dangling/unresolvable tag ref is skipped, same as before.
        let name_owned = name.to_owned();
        let Ok((object, target)) =
            tag::dereference(&reference, || ApiError::RefNotFound(name_owned.clone()))
        else {
            continue;
        };
        let annotation = tag
            .as_ref()
            .and_then(|tag| tag.message().ok().flatten())
            .and_then(first_line);
        let tagged_at = tag
            .as_ref()
            .and_then(|tag| tag.tagger())
            .and_then(|tagger| meta::git_time_to_zoned(tagger.when()))
            .map(|zoned| meta::format_rfc3339(&zoned));
        tags.push(TagRef {
            name: name.to_owned(),
            object,
            target,
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
