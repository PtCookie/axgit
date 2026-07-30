//! `GET /api/v1/repos/{repo}/blame/{ref}/{path...}` — per-line attribution.

use std::collections::HashMap;
use std::path::Path;

use git2::{BlameOptions, Commit, Oid, Repository};
use serde::Serialize;

use super::blob;
use super::commits::{CommitAuthor, signature_info, time_rfc3339};
use crate::error::ApiError;

/// One contiguous run of lines attributed to the same commit (docs/API.md).
#[derive(Debug, Serialize)]
pub struct BlameRange {
    /// 1-based, inclusive.
    pub start_line: usize,
    pub line_count: usize,
    pub sha: String,
    /// `None` for non-utf8 commit messages.
    pub summary: Option<String>,
    pub author: CommitAuthor,
    pub authored_at: Option<String>,
}

/// Response of the blame endpoint (docs/API.md).
#[derive(Debug, Serialize)]
pub struct BlameInfo {
    pub sha: String,
    pub path: String,
    pub binary: bool,
    pub too_large: bool,
    pub lines: usize,
    /// `start_line` ascending, covering the whole file with no gaps.
    /// Empty for `binary`, `too_large`, or empty files.
    pub ranges: Vec<BlameRange>,
}

/// Blames `path` as it exists in `commit`. Mirrors the blob endpoint's
/// binary/size classification (`repo::blob::classify`) so large or binary
/// files skip the (potentially expensive) blame walk entirely.
pub fn blame_file(repo: &Repository, commit: &Commit, path: &str) -> Result<BlameInfo, ApiError> {
    let (blob, _mode) = blob::blob_at(repo, commit, path)?;
    let (binary, too_large) = blob::classify(&blob);
    if binary || too_large {
        return Ok(BlameInfo {
            sha: commit.id().to_string(),
            path: path.to_owned(),
            binary,
            too_large,
            lines: 0,
            ranges: Vec::new(),
        });
    }

    // Path was already validated against this exact commit above, so a
    // failure here is unexpected — propagate as an internal error rather
    // than misreporting it as a missing ref/path.
    let mut options = BlameOptions::new();
    options.newest_commit(commit.id());
    let blame = repo.blame_file(Path::new(path), Some(&mut options))?;

    let mut commit_cache: HashMap<Oid, (Option<String>, CommitAuthor, Option<String>)> =
        HashMap::new();
    let mut ranges = Vec::with_capacity(blame.len());
    let mut lines = 0;
    for hunk in blame.iter() {
        // libgit2 reports a single zero-length hunk for an empty file; skip
        // it so an empty file yields `ranges: []` like binary/too_large.
        if hunk.lines_in_hunk() == 0 {
            continue;
        }
        let oid = hunk.final_commit_id();
        let (summary, author, authored_at) = commit_cache
            .entry(oid)
            .or_insert_with(|| {
                // A hunk's commit may be unreachable from the blamed commit only
                // if libgit2 itself is inconsistent; find_commit failing here
                // would be a genuine internal error, so `?` via the outer
                // function is appropriate — but we're inside a closure, so
                // fall back to the hunk's own signature on the rare miss.
                match repo.find_commit(oid) {
                    Ok(commit) => (
                        commit.summary().map(str::to_owned),
                        signature_info(&commit.author()),
                        time_rfc3339(commit.author().when()),
                    ),
                    Err(_) => (
                        None,
                        signature_info(&hunk.final_signature()),
                        time_rfc3339(hunk.final_signature().when()),
                    ),
                }
            })
            .clone();
        let line_count = hunk.lines_in_hunk();
        lines += line_count;
        ranges.push(BlameRange {
            start_line: hunk.final_start_line(),
            line_count,
            sha: oid.to_string(),
            summary,
            author,
            authored_at,
        });
    }
    ranges.sort_by_key(|range| range.start_line);

    Ok(BlameInfo {
        sha: commit.id().to_string(),
        path: path.to_owned(),
        binary: false,
        too_large: false,
        lines,
        ranges,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Commits `files` on top of `parent` using in-memory trees (no worktree).
    fn commit_files(repo: &Repository, parent: Option<Oid>, files: &[(&str, &str)]) -> Oid {
        let mut builder = repo.treebuilder(None).unwrap();
        for (name, content) in files {
            let blob = repo.blob(content.as_bytes()).unwrap();
            builder.insert(name, blob, 0o100644).unwrap();
        }
        let tree = repo.find_tree(builder.write().unwrap()).unwrap();
        let sig = git2::Signature::now("Test", "test@example.com").unwrap();
        let parents: Vec<_> = parent
            .map(|oid| repo.find_commit(oid).unwrap())
            .into_iter()
            .collect();
        let parent_refs: Vec<_> = parents.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, "test", &tree, &parent_refs)
            .unwrap()
    }

    #[test]
    fn blame_file_should_split_ranges_by_commit() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        // Only the last line changes, so the root's range stays contiguous
        // instead of being split around an untouched trailing line.
        let root = commit_files(&repo, None, &[("a.txt", "one\ntwo\n")]);
        let child = commit_files(&repo, Some(root), &[("a.txt", "one\nTWO\n")]);
        let commit = repo.find_commit(child).unwrap();

        let info = blame_file(&repo, &commit, "a.txt").unwrap();
        assert!(!info.binary);
        assert!(!info.too_large);
        assert_eq!(info.lines, 2);
        assert_eq!(info.ranges.len(), 2);
        assert_eq!(info.ranges[0].start_line, 1);
        assert_eq!(info.ranges[0].line_count, 1);
        assert_eq!(info.ranges[0].sha, root.to_string());
        assert_eq!(info.ranges[1].start_line, 2);
        assert_eq!(info.ranges[1].line_count, 1);
        assert_eq!(info.ranges[1].sha, child.to_string());
    }

    #[test]
    fn blame_file_should_return_path_not_found_for_missing_path() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let root = commit_files(&repo, None, &[("a.txt", "one\n")]);
        let commit = repo.find_commit(root).unwrap();

        assert!(matches!(
            blame_file(&repo, &commit, "missing.txt"),
            Err(ApiError::PathNotFound(path)) if path == "missing.txt"
        ));
    }
}
