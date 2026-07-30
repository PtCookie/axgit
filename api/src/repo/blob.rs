//! Blob reads for the blob and raw endpoints.

use std::path::Path;

use git2::{Blob, Commit, Repository};
use serde::Serialize;

use crate::error::ApiError;

/// Inline-content cap for blob/readme JSON; larger files are raw-only.
pub const BLOB_CONTENT_LIMIT: usize = 1024 * 1024;

/// Response of the blob endpoint (docs/API.md).
#[derive(Debug, Serialize)]
pub struct BlobInfo {
    /// Resolved commit sha the blob was read from.
    pub sha: String,
    pub path: String,
    /// Octal file mode, e.g. `"100644"` (`"120000"` for symlinks).
    pub mode: String,
    pub size: u64,
    pub binary: bool,
    pub too_large: bool,
    /// UTF-8 content; `None` when `binary` or `too_large`.
    pub content: Option<String>,
}

/// Raw blob bytes for the raw endpoint (owned — handlers cross thread bounds).
pub struct RawBlob {
    pub bytes: Vec<u8>,
    pub binary: bool,
}

/// Looks up `path` as a blob (regular file or symlink). Missing paths, trees,
/// and submodules are all `PathNotFound`.
pub(crate) fn blob_at<'r>(
    repo: &'r Repository,
    commit: &Commit,
    path: &str,
) -> Result<(Blob<'r>, i32), ApiError> {
    let not_found = || ApiError::PathNotFound(path.to_owned());
    if path.is_empty() {
        return Err(not_found());
    }
    let entry = commit
        .tree()?
        .get_path(Path::new(path))
        .map_err(|_| not_found())?;
    let mode = entry.filemode();
    let blob = entry
        .to_object(repo)
        .ok()
        .and_then(|object| object.into_blob().ok())
        .ok_or_else(not_found)?;
    Ok((blob, mode))
}

/// Binary/too-large classification shared by the blob, raw, and blame reads.
/// `too_large` short-circuits the utf8 check (large content is never
/// inlined, so its text-ness does not matter).
pub(crate) fn classify(blob: &Blob) -> (bool, bool) {
    let too_large = blob.content().len() > BLOB_CONTENT_LIMIT;
    let binary = blob.is_binary() || (!too_large && std::str::from_utf8(blob.content()).is_err());
    (binary, too_large)
}

pub fn read_blob(repo: &Repository, commit: &Commit, path: &str) -> Result<BlobInfo, ApiError> {
    let (blob, mode) = blob_at(repo, commit, path)?;
    let bytes = blob.content();
    let (binary, too_large) = classify(&blob);
    let content = (!binary && !too_large).then(|| {
        // Safe: `classify` already confirmed this is valid utf8 when not binary/too_large.
        String::from_utf8_lossy(bytes).into_owned()
    });
    Ok(BlobInfo {
        sha: commit.id().to_string(),
        path: path.to_owned(),
        mode: format!("{mode:06o}"),
        size: bytes.len() as u64,
        binary,
        too_large,
        content,
    })
}

pub fn read_raw(repo: &Repository, commit: &Commit, path: &str) -> Result<RawBlob, ApiError> {
    let (blob, _) = blob_at(repo, commit, path)?;
    Ok(RawBlob {
        bytes: blob.content().to_vec(),
        binary: blob.is_binary(),
    })
}
