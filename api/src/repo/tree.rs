//! Directory listing for `GET /api/v1/repos/{repo}/tree/{ref}/{path...}`.

use std::path::Path;

use git2::{Commit, Repository};
use serde::Serialize;
use utoipa::ToSchema;

use crate::error::ApiError;

const MODE_TREE: i32 = 0o040000;
const MODE_LINK: i32 = 0o120000;
const MODE_COMMIT: i32 = 0o160000; // gitlink (submodule)

/// Cap on a symlink target read inline — a target is a path, so PATH_MAX
/// (4096) is already generous; anything larger isn't a real link target.
/// Checked against the object header (a stat) so an oversized blob is never
/// loaded, and reported as `None` rather than truncated: half a path is a
/// *wrong* target, not a shorter one.
const SYMLINK_TARGET_LIMIT: u64 = 4096;

/// What a tree entry points at, derived from its file mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    Tree,
    Blob,
    /// Mode `120000`.
    Symlink,
    /// Gitlink (submodule), mode `160000`.
    Commit,
}

/// Tree entry of the tree endpoint (docs/API.md).
#[derive(Debug, Serialize, ToSchema)]
pub struct TreeEntryInfo {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: EntryKind,
    /// Octal file mode, e.g. `"100644"`.
    #[schema(example = "100644")]
    pub mode: String,
    /// Object size in bytes; blobs only, `None` otherwise.
    #[schema(required = true)]
    pub size: Option<u64>,
    /// Symlink target path; symlinks only, `None` otherwise — also `None` for
    /// a non-UTF-8 target or one past `SYMLINK_TARGET_LIMIT`. Relative to the
    /// entry's own directory, exactly as stored; clients resolve it.
    #[schema(required = true)]
    pub target: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TreeListing {
    /// Resolved commit sha the listing was taken from.
    pub sha: String,
    /// Requested directory path; empty string for the root tree.
    pub path: String,
    /// Trees first, then by name ascending.
    pub entries: Vec<TreeEntryInfo>,
}

fn kind_of(mode: i32) -> EntryKind {
    match mode {
        MODE_TREE => EntryKind::Tree,
        MODE_LINK => EntryKind::Symlink,
        MODE_COMMIT => EntryKind::Commit,
        _ => EntryKind::Blob,
    }
}

/// Lists the tree at `path` (empty = root), trees first then by name.
pub fn list_tree(repo: &Repository, commit: &Commit, path: &str) -> Result<TreeListing, ApiError> {
    let root = commit.tree()?;
    let tree = if path.is_empty() {
        root
    } else {
        root.get_path(Path::new(path))
            .ok()
            .and_then(|entry| entry.to_object(repo).ok())
            .and_then(|object| object.into_tree().ok())
            // Missing path and blob-at-path both land here: not a directory.
            .ok_or_else(|| ApiError::PathNotFound(path.to_owned()))?
    };

    let odb = repo.odb()?;
    let mut entries = Vec::new();
    for entry in tree.iter() {
        let Some(name) = entry.name() else {
            continue; // non-utf8 entry name
        };
        let mode = entry.filemode();
        let kind = kind_of(mode);
        // read_header stats the object without loading its content — it backs
        // both a blob's reported size and the symlink target's size gate.
        let object_size = matches!(kind, EntryKind::Blob | EntryKind::Symlink)
            .then(|| odb.read_header(entry.id()).ok())
            .flatten()
            .map(|(size, _)| size as u64);
        let size = (kind == EntryKind::Blob).then_some(object_size).flatten();
        // A symlink's target is its blob content. Non-UTF-8 collapses to
        // `None`, same rule `blob.rs::classify` applies to blob content.
        let target = match kind {
            EntryKind::Symlink if object_size.is_some_and(|size| size <= SYMLINK_TARGET_LIMIT) => {
                entry
                    .to_object(repo)
                    .ok()
                    .and_then(|object| object.into_blob().ok())
                    .and_then(|blob| str::from_utf8(blob.content()).ok().map(str::to_owned))
            }
            _ => None,
        };
        entries.push(TreeEntryInfo {
            name: name.to_owned(),
            kind,
            mode: format!("{mode:06o}"),
            size,
            target,
        });
    }
    entries.sort_by(|a, b| {
        (a.kind != EntryKind::Tree)
            .cmp(&(b.kind != EntryKind::Tree))
            .then_with(|| a.name.cmp(&b.name))
    });

    Ok(TreeListing {
        sha: commit.id().to_string(),
        path: path.to_owned(),
        entries,
    })
}
