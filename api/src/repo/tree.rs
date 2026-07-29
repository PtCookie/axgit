//! Directory listing for `GET /api/v1/repos/{repo}/tree/{ref}/{path...}`.

use std::path::Path;

use git2::{Commit, Repository};
use serde::Serialize;

use crate::error::ApiError;

const MODE_TREE: i32 = 0o040000;
const MODE_LINK: i32 = 0o120000;
const MODE_COMMIT: i32 = 0o160000; // gitlink (submodule)

/// Tree entry of the tree endpoint (docs/API.md).
#[derive(Debug, Serialize)]
pub struct TreeEntryInfo {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: &'static str,
    /// Octal file mode, e.g. `"100644"`.
    pub mode: String,
    /// Object size in bytes; blobs only, `None` otherwise.
    pub size: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct TreeListing {
    /// Resolved commit sha the listing was taken from.
    pub sha: String,
    pub path: String,
    pub entries: Vec<TreeEntryInfo>,
}

fn kind_of(mode: i32) -> &'static str {
    match mode {
        MODE_TREE => "tree",
        MODE_LINK => "symlink",
        MODE_COMMIT => "commit",
        _ => "blob",
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
        // read_header stats the object without loading its content.
        let size = (kind == "blob")
            .then(|| odb.read_header(entry.id()).ok())
            .flatten()
            .map(|(size, _)| size as u64);
        entries.push(TreeEntryInfo {
            name: name.to_owned(),
            kind,
            mode: format!("{mode:06o}"),
            size,
        });
    }
    entries.sort_by(|a, b| {
        (a.kind != "tree")
            .cmp(&(b.kind != "tree"))
            .then_with(|| a.name.cmp(&b.name))
    });

    Ok(TreeListing {
        sha: commit.id().to_string(),
        path: path.to_owned(),
        entries,
    })
}
