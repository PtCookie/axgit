use std::path::Path;

use git2::{Commit, Oid, Repository};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{diff, meta};
use crate::error::ApiError;

/// Commit author of `GET /api/v1/repos/{repo}/commits` (docs/API.md).
/// The raw email is never exposed; `email_hash` seeds locally generated avatars.
#[derive(Debug, Serialize)]
pub struct CommitAuthor {
    pub name: String,
    /// sha256 hex of the trimmed, lowercased author email.
    pub email_hash: String,
}

/// Log entry of `GET /api/v1/repos/{repo}/commits` (docs/API.md).
#[derive(Debug, Serialize)]
pub struct CommitInfo {
    pub sha: String,
    /// `None` for non-utf8 commit messages.
    pub summary: Option<String>,
    pub author: CommitAuthor,
    /// Authordate (RFC 3339).
    pub authored_at: Option<String>,
    pub parents: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct CommitsPage {
    pub commits: Vec<CommitInfo>,
    /// Sha of the first commit of the next page; `null` on the last page.
    pub next_cursor: Option<String>,
}

/// Commit detail of `GET /api/v1/repos/{repo}/commits/{sha}` (docs/API.md).
/// Superset of [`CommitInfo`] so the frontend can extend the log entry type.
#[derive(Debug, Serialize)]
pub struct CommitDetail {
    pub sha: String,
    pub summary: Option<String>,
    /// Full commit message. `None` for non-utf8 messages.
    pub message: Option<String>,
    pub author: CommitAuthor,
    pub committer: CommitAuthor,
    pub authored_at: Option<String>,
    pub committed_at: Option<String>,
    pub parents: Vec<String>,
    /// First-parent diffstat (docs/API.md).
    pub diffstat: diff::DiffStat,
}

/// Builds the commit detail response, including the first-parent diffstat.
pub fn detail(repo: &Repository, commit: &Commit) -> Result<CommitDetail, ApiError> {
    Ok(CommitDetail {
        sha: commit.id().to_string(),
        summary: commit.summary().map(str::to_owned),
        message: commit.message().map(str::to_owned),
        author: signature_info(&commit.author()),
        committer: signature_info(&commit.committer()),
        authored_at: time_rfc3339(commit.author().when()),
        committed_at: time_rfc3339(commit.committer().when()),
        parents: commit.parent_ids().map(|id| id.to_string()).collect(),
        diffstat: diff::diffstat(repo, commit)?,
    })
}

/// Walks history from `start` (inclusive) and returns up to `limit` commits,
/// keeping only those touching `path` when given. libgit2's default walk order
/// is the closest match to `git log`, so no explicit sorting is set.
pub fn log(
    repo: &Repository,
    start: Oid,
    path: Option<&Path>,
    limit: usize,
) -> Result<CommitsPage, ApiError> {
    let mut revwalk = repo.revwalk()?;
    revwalk.push(start)?;
    let mut commits = Vec::new();
    let mut next_cursor = None;
    for oid in revwalk {
        let oid = oid?;
        let commit = repo.find_commit(oid)?;
        if let Some(path) = path
            && !touches_path(&commit, path)
        {
            continue;
        }
        if commits.len() == limit {
            next_cursor = Some(oid.to_string());
            break;
        }
        commits.push(commit_info(&commit));
    }
    Ok(CommitsPage {
        commits,
        next_cursor,
    })
}

fn commit_info(commit: &Commit) -> CommitInfo {
    CommitInfo {
        sha: commit.id().to_string(),
        summary: commit.summary().map(str::to_owned),
        author: signature_info(&commit.author()),
        authored_at: time_rfc3339(commit.author().when()),
        parents: commit.parent_ids().map(|id| id.to_string()).collect(),
    }
}

fn signature_info(signature: &git2::Signature) -> CommitAuthor {
    CommitAuthor {
        name: String::from_utf8_lossy(signature.name_bytes()).into_owned(),
        email_hash: email_hash(signature.email_bytes()),
    }
}

fn time_rfc3339(time: git2::Time) -> Option<String> {
    meta::git_time_to_zoned(time).map(|zoned| meta::format_rfc3339(&zoned))
}

/// Whether the commit changed `path` relative to its parents. Approximates
/// `git log -- <path>` history simplification by entry-id comparison: a merge
/// commit is included only when the path differs from *every* parent. Unlike
/// git we do not prune the walk to a TREESAME parent, so a few side-branch
/// commits git would hide may still appear.
fn touches_path(commit: &Commit, path: &Path) -> bool {
    let entry = path_entry_id(commit, path);
    if commit.parent_count() == 0 {
        return entry.is_some();
    }
    commit
        .parents()
        .all(|parent| path_entry_id(&parent, path) != entry)
}

/// Object id of the tree entry at `path` (file or directory); `None` when absent.
fn path_entry_id(commit: &Commit, path: &Path) -> Option<Oid> {
    let tree = commit.tree().ok()?;
    tree.get_path(path).ok().map(|entry| entry.id())
}

/// Trimmed + lowercased (gravatar-style) so the avatar seed is stable across
/// case/whitespace variants of the same address.
fn email_hash(email: &[u8]) -> String {
    let normalized = String::from_utf8_lossy(email).trim().to_lowercase();
    format!("{:x}", Sha256::digest(normalized.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_hash_should_normalize_case_and_whitespace() {
        let hash = email_hash(b"author@example.com");
        assert_eq!(hash.len(), 64);
        assert_eq!(email_hash(b" Author@Example.COM "), hash);
    }

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
    fn touches_path_should_track_entry_changes() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let root = commit_files(&repo, None, &[("a.txt", "one")]);
        let child = commit_files(&repo, Some(root), &[("a.txt", "one"), ("b.txt", "two")]);
        let root = repo.find_commit(root).unwrap();
        let child = repo.find_commit(child).unwrap();

        // Root commit: included only for paths it introduces.
        assert!(touches_path(&root, Path::new("a.txt")));
        assert!(!touches_path(&root, Path::new("b.txt")));
        // Child adds b.txt but leaves a.txt untouched.
        assert!(touches_path(&child, Path::new("b.txt")));
        assert!(!touches_path(&child, Path::new("a.txt")));
        assert!(!touches_path(&child, Path::new("missing.txt")));
    }
}
