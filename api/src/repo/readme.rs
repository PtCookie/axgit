//! README discovery for `GET /api/v1/repos/{repo}/readme`.

use git2::{Commit, Repository};
use serde::Serialize;

use super::blob::BLOB_CONTENT_LIMIT;
use crate::error::ApiError;

/// Candidate file names in priority order, with their render format.
const CANDIDATES: [(&str, &str); 4] = [
    ("README.md", "markdown"),
    ("README.rst", "rst"),
    ("README.txt", "plain"),
    ("README", "plain"),
];

const MODE_LINK: i32 = 0o120000;

#[derive(Debug, Serialize)]
pub struct ReadmeInfo {
    /// Actual file name as it appears in the tree (case may differ).
    pub path: String,
    pub format: &'static str,
    pub content: String,
}

/// Finds the first usable README in the commit's root tree. Names match
/// case-insensitively; symlinks and binary, non-UTF-8, or oversized
/// candidates are skipped.
pub fn find_readme(repo: &Repository, commit: &Commit) -> Result<ReadmeInfo, ApiError> {
    let tree = commit.tree()?;
    for (candidate, format) in CANDIDATES {
        for entry in tree.iter() {
            let Some(name) = entry.name() else {
                continue; // non-utf8 entry name
            };
            if !name.eq_ignore_ascii_case(candidate) || entry.filemode() == MODE_LINK {
                continue;
            }
            let Some(blob) = entry
                .to_object(repo)
                .ok()
                .and_then(|object| object.into_blob().ok())
            else {
                continue; // tree or submodule that happens to share the name
            };
            if blob.content().len() > BLOB_CONTENT_LIMIT {
                continue;
            }
            let Ok(content) = std::str::from_utf8(blob.content()) else {
                continue; // binary
            };
            return Ok(ReadmeInfo {
                path: name.to_owned(),
                format,
                content: content.to_owned(),
            });
        }
    }
    Err(ApiError::PathNotFound("README".to_owned()))
}
