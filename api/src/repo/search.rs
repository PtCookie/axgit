//! Repository search: content, file path, and commit message
//! (`GET /api/v1/repos/{repo}/search`, docs/API.md).
//!
//! git2 in-process scan (docs/DECISIONS.md #26) rather than a `git grep` exec
//! or a persistent index: the container's read-only, stateless invariants
//! (CLAUDE.md) rule out an index, and libgit2 lets every pass enforce an
//! exact byte/file/commit budget — a `git grep` exec could only be bounded by
//! a process timeout. Same reasoning as blame choosing git2 over exec
//! (DECISIONS.md #14).

use git2::{Commit, ObjectType, Oid, Repository, TreeWalkMode, TreeWalkResult};
use serde::Serialize;
use utoipa::ToSchema;

use super::blob;
use super::commits::{self, CommitInfo};
use crate::error::ApiError;

/// Regular-file mode; mirrors `repo/tree.rs`'s private constant (not shared —
/// each module's blob/tree classification stays self-contained).
const MODE_LINK: i32 = 0o120000;

/// Tree entries considered per search — a scan-time budget independent of
/// `limit` (the result-count cap), so a huge non-matching tree can't run
/// unbounded.
const MAX_SCANNED_BLOBS: usize = 20_000;
/// Cumulative blob bytes actually read for `type=content`, checked against
/// each blob's header size before it is loaded.
const MAX_SCANNED_BYTES: usize = 32 * 1024 * 1024;
/// Matching lines kept per file.
const MAX_MATCHES_PER_FILE: usize = 10;
/// Characters kept per matched line (char-boundary safe truncation).
const MAX_LINE_CHARS: usize = 500;
/// Commits walked for `type=message`.
const MAX_SCANNED_COMMITS: usize = 10_000;

/// Requested search variant, echoed back in the response's `type` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum SearchKind {
    /// File content, line by line.
    Content,
    /// File path (name), no content read.
    Path,
    /// Commit message (title + body).
    Message,
}

/// Matched line within a file (docs/API.md).
#[derive(Debug, Serialize, ToSchema)]
pub struct LineMatch {
    /// 1-based line number.
    pub line: usize,
    /// The matching line, truncated to [`MAX_LINE_CHARS`] characters.
    pub text: String,
}

/// A file with at least one match (docs/API.md).
#[derive(Debug, Serialize, ToSchema)]
pub struct FileMatch {
    pub path: String,
    /// Empty for `type=path` matches — the path itself is the match, there is
    /// no line to point at.
    pub lines: Vec<LineMatch>,
}

/// Response of `GET /api/v1/repos/{repo}/search` (docs/API.md).
#[derive(Debug, Serialize, ToSchema)]
pub struct SearchResults {
    /// Resolved commit sha the search ran against. `None` only for an empty
    /// repository (unborn HEAD) with no explicit `ref`.
    #[schema(required = true)]
    pub sha: Option<String>,
    #[serde(rename = "type")]
    pub kind: SearchKind,
    /// `true` when a scan or result budget was hit before the search
    /// finished — the results are a prefix, not necessarily the complete
    /// match set.
    pub truncated: bool,
    /// `content`/`path` results; empty for `type=message`.
    pub files: Vec<FileMatch>,
    /// `message` results; empty otherwise. Reuses the commit log's shape so
    /// the frontend can render matches the same way as log rows.
    pub commits: Vec<CommitInfo>,
}

/// Runs a search of `kind` for `query` (already trimmed/length-checked by the
/// caller) against `commit`, keeping at most `limit` results.
pub fn search(
    repo: &Repository,
    commit: &Commit,
    kind: SearchKind,
    query: &str,
    limit: usize,
) -> Result<SearchResults, ApiError> {
    let query_lower = query.to_lowercase();
    let (files, commits, truncated) = match kind {
        SearchKind::Content => {
            let (files, truncated) = search_content(repo, commit, &query_lower, limit)?;
            (files, Vec::new(), truncated)
        }
        SearchKind::Path => {
            let (files, truncated) = search_paths(commit, &query_lower, limit)?;
            (files, Vec::new(), truncated)
        }
        SearchKind::Message => {
            let (commits, truncated) = search_messages(repo, commit, &query_lower, limit)?;
            (Vec::new(), commits, truncated)
        }
    };
    Ok(SearchResults {
        sha: Some(commit.id().to_string()),
        kind,
        truncated,
        files,
        commits,
    })
}

/// A scanned tree entry: `(path, blob oid, filemode)`.
type BlobEntry = (String, Oid, i32);

/// Collects up to [`MAX_SCANNED_BLOBS`] blob entries from `commit`'s tree,
/// depth-first. Trees are descended into but not yielded; gitlinks
/// (submodules) are skipped entirely — there is no content to search and a
/// path match on a gitlink isn't useful. Entries with a non-utf8 name are
/// skipped (the JSON `path` field cannot represent them).
fn scan_blobs(commit: &Commit) -> Result<(Vec<BlobEntry>, bool), ApiError> {
    let tree = commit.tree()?;
    let mut entries = Vec::new();
    let mut truncated = false;
    let result = tree.walk(TreeWalkMode::PreOrder, |root, entry| {
        if entries.len() >= MAX_SCANNED_BLOBS {
            truncated = true;
            return TreeWalkResult::Abort;
        }
        if entry.kind() != Some(ObjectType::Blob) {
            return TreeWalkResult::Ok;
        }
        let Some(name) = entry.name() else {
            return TreeWalkResult::Ok;
        };
        entries.push((format!("{root}{name}"), entry.id(), entry.filemode()));
        TreeWalkResult::Ok
    });
    // `Abort` makes `walk` return an error (libgit2's GIT_EUSER); that's our
    // own budget signal, not a genuine failure — only propagate an error that
    // didn't come from our own abort.
    if let Err(err) = result
        && !truncated
    {
        return Err(err.into());
    }
    Ok((entries, truncated))
}

/// Case-insensitive substring test; `needle_lower` must already be
/// lowercased (the caller lowercases the query once, not per line/path/
/// commit).
fn contains_ignore_case(haystack: &str, needle_lower: &str) -> bool {
    if haystack.is_ascii() {
        haystack.to_ascii_lowercase().contains(needle_lower)
    } else {
        haystack.to_lowercase().contains(needle_lower)
    }
}

/// Truncates `s` to at most `max_chars` characters, respecting char
/// boundaries (never splits a multi-byte codepoint).
fn truncate_chars(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

fn search_content(
    repo: &Repository,
    commit: &Commit,
    query_lower: &str,
    limit: usize,
) -> Result<(Vec<FileMatch>, bool), ApiError> {
    let (entries, mut truncated) = scan_blobs(commit)?;
    let odb = repo.odb()?;
    let mut files = Vec::new();
    let mut scanned_bytes: usize = 0;
    for (path, oid, mode) in entries {
        if files.len() >= limit {
            truncated = true;
            break;
        }
        // Symlink content is a target path, not file text — excluded here
        // (still eligible for `type=path`, `scan_blobs` doesn't filter it).
        if mode == MODE_LINK {
            continue;
        }
        // Cheap header read: skip oversized blobs and stop once the byte
        // budget is spent, before ever loading content into memory.
        let Ok((size, _)) = odb.read_header(oid) else {
            continue;
        };
        if size > blob::BLOB_CONTENT_LIMIT {
            continue;
        }
        if scanned_bytes + size > MAX_SCANNED_BYTES {
            truncated = true;
            break;
        }
        let blob_obj = repo.find_blob(oid)?;
        scanned_bytes += blob_obj.content().len();
        let (binary, too_large) = blob::classify(&blob_obj);
        if binary || too_large {
            continue;
        }
        // Safe: `classify` already confirmed this is valid utf8 when not
        // binary/too_large (same guarantee `blob::read_blob` relies on).
        let text = String::from_utf8_lossy(blob_obj.content());
        let mut lines = Vec::new();
        for (idx, line) in text.lines().enumerate() {
            if contains_ignore_case(line, query_lower) {
                lines.push(LineMatch {
                    line: idx + 1,
                    text: truncate_chars(line, MAX_LINE_CHARS),
                });
                if lines.len() >= MAX_MATCHES_PER_FILE {
                    break;
                }
            }
        }
        if !lines.is_empty() {
            files.push(FileMatch { path, lines });
        }
    }
    Ok((files, truncated))
}

fn search_paths(
    commit: &Commit,
    query_lower: &str,
    limit: usize,
) -> Result<(Vec<FileMatch>, bool), ApiError> {
    let (entries, mut truncated) = scan_blobs(commit)?;
    let mut files = Vec::new();
    for (path, _oid, _mode) in entries {
        if files.len() >= limit {
            truncated = true;
            break;
        }
        if contains_ignore_case(&path, query_lower) {
            files.push(FileMatch {
                path,
                lines: Vec::new(),
            });
        }
    }
    Ok((files, truncated))
}

fn search_messages(
    repo: &Repository,
    commit: &Commit,
    query_lower: &str,
    limit: usize,
) -> Result<(Vec<CommitInfo>, bool), ApiError> {
    let mut revwalk = repo.revwalk()?;
    revwalk.push(commit.id())?;
    let mut commits = Vec::new();
    let mut truncated = false;
    for (scanned, oid) in revwalk.enumerate() {
        if commits.len() >= limit {
            truncated = true;
            break;
        }
        if scanned >= MAX_SCANNED_COMMITS {
            truncated = true;
            break;
        }
        let oid = oid?;
        let found = repo.find_commit(oid)?;
        // Non-utf8 messages can't match a text query; skip rather than error.
        if let Some(message) = found.message()
            && contains_ignore_case(message, query_lower)
        {
            commits.push(commits::commit_info(&found));
        }
    }
    Ok((commits, truncated))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Commits `files` on top of `parent` using in-memory trees (no
    /// worktree). Mirrors the helper in `blame.rs`/`commits.rs`.
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
    fn contains_ignore_case_should_ignore_case() {
        assert!(contains_ignore_case("Hello World", "world"));
        assert!(contains_ignore_case("Hello World", "hello"));
        assert!(!contains_ignore_case("Hello World", "bye"));
        assert!(contains_ignore_case("héllo", "héllo"));
    }

    #[test]
    fn truncate_chars_should_respect_char_boundaries() {
        assert_eq!(truncate_chars("hello", 3), "hel");
        assert_eq!(truncate_chars("hello", 10), "hello");
        assert_eq!(truncate_chars("héllo", 2), "hé");
    }

    #[test]
    fn search_content_should_find_matching_lines() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let root = commit_files(
            &repo,
            None,
            &[
                ("a.txt", "one\nTWO fish\nthree\n"),
                ("b.txt", "no match here\n"),
            ],
        );
        let commit = repo.find_commit(root).unwrap();

        let results = search(&repo, &commit, SearchKind::Content, "two", 50).unwrap();
        assert_eq!(results.sha, Some(root.to_string()));
        assert_eq!(results.kind, SearchKind::Content);
        assert!(!results.truncated);
        assert_eq!(results.files.len(), 1);
        assert_eq!(results.files[0].path, "a.txt");
        assert_eq!(results.files[0].lines.len(), 1);
        assert_eq!(results.files[0].lines[0].line, 2);
        assert_eq!(results.files[0].lines[0].text, "TWO fish");
    }

    #[test]
    fn search_content_should_skip_binary_files() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let root = commit_files(&repo, None, &[("bin.dat", "text\0with-nul-needle\n")]);
        let commit = repo.find_commit(root).unwrap();

        let results = search(&repo, &commit, SearchKind::Content, "needle", 50).unwrap();
        assert!(results.files.is_empty());
    }

    #[test]
    fn search_path_should_match_full_path_case_insensitively() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        // `commit_files` builds a flat tree (no `/` in a name — see its own
        // doc comment); nested paths are covered by the integration test
        // (`api/tests/search_test.rs`), which uses real subdirectories.
        let root = commit_files(
            &repo,
            None,
            &[("Main.rs", "fn main() {}"), ("README.md", "hi")],
        );
        let commit = repo.find_commit(root).unwrap();

        let results = search(&repo, &commit, SearchKind::Path, "main", 50).unwrap();
        assert_eq!(results.files.len(), 1);
        assert_eq!(results.files[0].path, "Main.rs");
        assert!(results.files[0].lines.is_empty());
    }

    #[test]
    fn search_path_should_apply_the_limit_and_report_truncation() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let files: Vec<(&str, &str)> = vec![
            ("match-a.txt", "x"),
            ("match-b.txt", "x"),
            ("match-c.txt", "x"),
        ];
        let root = commit_files(&repo, None, &files);
        let commit = repo.find_commit(root).unwrap();

        let results = search(&repo, &commit, SearchKind::Path, "match", 2).unwrap();
        assert_eq!(results.files.len(), 2);
        assert!(results.truncated);
    }

    #[test]
    fn search_message_should_find_matching_commits() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let root = commit_files(&repo, None, &[("a.txt", "one")]);
        // A second commit with a controlled message (`commit_files` hardcodes
        // "test"), on top of `root` — `Some("HEAD")` requires the current tip
        // as a parent, or git2 rejects the ref update.
        let sig = git2::Signature::now("Test", "test@example.com").unwrap();
        let root_commit = repo.find_commit(root).unwrap();
        let tree = root_commit.tree().unwrap();
        let renamed = repo
            .commit(
                Some("HEAD"),
                &sig,
                &sig,
                "fix: correct the frobnicator",
                &tree,
                &[&root_commit],
            )
            .unwrap();
        let unrelated = commit_files(&repo, Some(renamed), &[("a.txt", "two")]);
        let commit = repo.find_commit(unrelated).unwrap();

        let results = search(&repo, &commit, SearchKind::Message, "frobnicator", 50).unwrap();
        assert_eq!(results.commits.len(), 1);
        assert_eq!(results.commits[0].sha, renamed.to_string());
        assert!(results.files.is_empty());
    }
}
