//! Diffstat and structured diff for a single commit (docs/API.md).
//!
//! All diffs are first-parent: a merge shows its changes against parent 0 and
//! the root commit diffs against the empty tree. Rename detection runs with
//! libgit2 defaults on both entry points so the diffstat and the diff always
//! agree on the file list.

use std::path::Path;

use git2::{Commit, Delta, Diff, DiffDelta, DiffOptions, Patch, Repository};
use serde::Serialize;
use utoipa::ToSchema;

use crate::error::ApiError;

/// Per-file cap on rendered diff lines. Hunks are dropped whole once the cap
/// would be crossed — a hunk is never cut in the middle.
pub const MAX_FILE_DIFF_LINES: usize = 1000;
/// Cap on files rendered by the diff endpoint. The diffstat has no cap, so the
/// full file list stays available on the commit detail response.
pub const MAX_DIFF_FILES: usize = 300;

/// How a file changed between the two trees. Rename detection runs with
/// libgit2 defaults (renames only, 50% similarity).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum DiffStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
    Copied,
    Typechange,
}

/// Diffstat of `GET /api/v1/repos/{repo}/commits/{sha}` (docs/API.md).
#[derive(Debug, Serialize, ToSchema)]
pub struct DiffStat {
    pub files: Vec<DiffStatFile>,
    pub files_changed: usize,
    pub total_additions: usize,
    pub total_deletions: usize,
}

/// Per-file entry shared by the diffstat and the diff (`#[serde(flatten)]`).
#[derive(Debug, Serialize, ToSchema)]
pub struct DiffStatFile {
    #[schema(example = "src/main.rs")]
    pub path: String,
    /// Previous path; only set for `renamed`/`copied`.
    #[schema(required = true)]
    pub old_path: Option<String>,
    pub status: DiffStatus,
    /// Always the full count, even when the file's hunks were truncated.
    /// Binary files report 0.
    pub additions: usize,
    /// Always the full count, even when the file's hunks were truncated.
    /// Binary files report 0.
    pub deletions: usize,
    pub binary: bool,
}

/// Structured diff of `GET /api/v1/repos/{repo}/commits/{sha}/diff` (docs/API.md).
#[derive(Debug, Serialize, ToSchema)]
pub struct CommitDiff {
    pub sha: String,
    /// First parent the diff was computed against; `null` for a root commit.
    #[schema(required = true)]
    pub parent: Option<String>,
    /// `true` when files beyond [`MAX_DIFF_FILES`] were omitted.
    pub truncated: bool,
    pub files: Vec<FileDiff>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FileDiff {
    #[serde(flatten)]
    pub stat: DiffStatFile,
    /// `true` when hunks were dropped past [`MAX_FILE_DIFF_LINES`].
    /// `additions`/`deletions` still count the full change.
    pub truncated: bool,
    /// Empty for binary files.
    pub hunks: Vec<Hunk>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Hunk {
    /// `@@ -a,b +c,d @@ <context>` line without the trailing newline.
    #[schema(example = "@@ -1,2 +1,2 @@")]
    pub header: String,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<Line>,
}

/// Which side of the diff a line belongs to. Other libgit2 origins (EOF
/// newline markers and the like) are dropped rather than reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
pub enum LineOrigin {
    #[serde(rename = " ")]
    Context,
    #[serde(rename = "+")]
    Addition,
    #[serde(rename = "-")]
    Deletion,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Line {
    pub origin: LineOrigin,
    /// Line content without the trailing newline (lossy utf-8).
    pub content: String,
    /// `null` on added lines.
    #[schema(required = true)]
    pub old_lineno: Option<u32>,
    /// `null` on deleted lines.
    #[schema(required = true)]
    pub new_lineno: Option<u32>,
}

/// Diffstat against the first parent, covering every changed file.
pub fn diffstat(repo: &Repository, commit: &Commit) -> Result<DiffStat, ApiError> {
    let diff = build_diff(repo, commit, None)?;
    let mut files = Vec::with_capacity(diff.deltas().len());
    let (mut total_additions, mut total_deletions) = (0, 0);
    for idx in 0..diff.deltas().len() {
        let (_, file) = load_file(&diff, idx)?;
        total_additions += file.additions;
        total_deletions += file.deletions;
        files.push(file);
    }
    Ok(DiffStat {
        files_changed: files.len(),
        total_additions,
        total_deletions,
        files,
    })
}

/// Structured diff against the first parent, optionally limited to `path`.
/// A path absent from the commit's changes yields an empty file list, not an
/// error — same contract as the log endpoint's `path` filter.
pub fn commit_diff(
    repo: &Repository,
    commit: &Commit,
    path: Option<&Path>,
) -> Result<CommitDiff, ApiError> {
    let diff = build_diff(repo, commit, path)?;
    let delta_count = diff.deltas().len();
    let rendered = delta_count.min(MAX_DIFF_FILES);
    let mut files = Vec::with_capacity(rendered);
    for idx in 0..rendered {
        let (patch, stat) = load_file(&diff, idx)?;
        let (hunks, truncated) = match patch {
            Some(mut patch) if !stat.binary => collect_hunks(&mut patch)?,
            _ => (Vec::new(), false),
        };
        files.push(FileDiff {
            stat,
            truncated,
            hunks,
        });
    }
    Ok(CommitDiff {
        sha: commit.id().to_string(),
        parent: commit.parent_id(0).ok().map(|id| id.to_string()),
        truncated: delta_count > MAX_DIFF_FILES,
        files,
    })
}

/// First-parent tree diff with rename detection; `None` old side (root commit)
/// is the empty tree.
fn build_diff<'r>(
    repo: &'r Repository,
    commit: &Commit<'_>,
    path: Option<&Path>,
) -> Result<Diff<'r>, ApiError> {
    let new_tree = commit.tree()?;
    let old_tree = if commit.parent_count() > 0 {
        Some(commit.parent(0)?.tree()?)
    } else {
        None
    };
    let mut opts = DiffOptions::new();
    if let Some(path) = path {
        // Literal single-path restriction; glob matching is not part of the
        // API contract.
        opts.pathspec(path);
        opts.disable_pathspec_match(true);
    }
    let mut diff = repo.diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), Some(&mut opts))?;
    // libgit2 defaults: renames only (no copies), 50% similarity.
    diff.find_similar(None)?;
    Ok(diff)
}

/// Loads the patch for delta `idx` and summarizes it. The patch is returned so
/// hunk collection reuses the (expensive) content load; a `None` patch means
/// libgit2 produced no text for the delta and it is reported as binary.
fn load_file<'d>(
    diff: &Diff<'d>,
    idx: usize,
) -> Result<(Option<Patch<'d>>, DiffStatFile), ApiError> {
    match Patch::from_diff(diff, idx)? {
        Some(patch) => {
            let file = {
                let delta = patch.delta();
                let (path, old_path) = delta_paths(&delta);
                // The binary flag is only reliable after content is loaded,
                // which Patch::from_diff just did.
                let binary = delta.flags().is_binary();
                let (_, additions, deletions) = patch.line_stats()?;
                DiffStatFile {
                    path,
                    old_path,
                    status: diff_status(delta.status()),
                    additions,
                    deletions,
                    binary,
                }
            };
            Ok((Some(patch), file))
        }
        None => {
            let delta = diff
                .get_delta(idx)
                .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("diff delta {idx} vanished")))?;
            let (path, old_path) = delta_paths(&delta);
            Ok((
                None,
                DiffStatFile {
                    path,
                    old_path,
                    status: diff_status(delta.status()),
                    additions: 0,
                    deletions: 0,
                    binary: true,
                },
            ))
        }
    }
}

/// Collects hunks up to [`MAX_FILE_DIFF_LINES`]; returns `(hunks, truncated)`.
fn collect_hunks(patch: &mut Patch) -> Result<(Vec<Hunk>, bool), ApiError> {
    let mut hunks = Vec::new();
    let mut rendered = 0usize;
    for hunk_idx in 0..patch.num_hunks() {
        let (line_count, header, old_start, old_lines, new_start, new_lines) = {
            let (hunk, line_count) = patch.hunk(hunk_idx)?;
            (
                line_count,
                trim_newline(&String::from_utf8_lossy(hunk.header())).to_owned(),
                hunk.old_start(),
                hunk.old_lines(),
                hunk.new_start(),
                hunk.new_lines(),
            )
        };
        if rendered + line_count > MAX_FILE_DIFF_LINES {
            return Ok((hunks, true));
        }
        rendered += line_count;
        let mut lines = Vec::with_capacity(line_count);
        for line_idx in 0..line_count {
            let line = patch.line_in_hunk(hunk_idx, line_idx)?;
            let Some(origin) = line_origin(line.origin()) else {
                continue;
            };
            lines.push(Line {
                origin,
                content: trim_newline(&String::from_utf8_lossy(line.content())).to_owned(),
                old_lineno: line.old_lineno(),
                new_lineno: line.new_lineno(),
            });
        }
        hunks.push(Hunk {
            header,
            old_start,
            old_lines,
            new_start,
            new_lines,
            lines,
        });
    }
    Ok((hunks, false))
}

fn delta_paths(delta: &DiffDelta) -> (String, Option<String>) {
    let lossy = |path: &Path| path.to_string_lossy().into_owned();
    let path = delta
        .new_file()
        .path()
        .or_else(|| delta.old_file().path())
        .map(lossy)
        .unwrap_or_default();
    let old_path = matches!(delta.status(), Delta::Renamed | Delta::Copied)
        .then(|| delta.old_file().path().map(lossy))
        .flatten();
    (path, old_path)
}

fn diff_status(status: Delta) -> DiffStatus {
    match status {
        Delta::Added => DiffStatus::Added,
        Delta::Deleted => DiffStatus::Deleted,
        Delta::Modified => DiffStatus::Modified,
        Delta::Renamed => DiffStatus::Renamed,
        Delta::Copied => DiffStatus::Copied,
        Delta::Typechange => DiffStatus::Typechange,
        // Worktree/index-only states cannot appear in a tree-to-tree diff.
        Delta::Unmodified
        | Delta::Ignored
        | Delta::Untracked
        | Delta::Unreadable
        | Delta::Conflicted => DiffStatus::Modified,
    }
}

/// Maps a libgit2 line origin onto the three the API contract exposes; other
/// origins (EOF newline markers, file headers) yield `None` and are dropped.
fn line_origin(origin: char) -> Option<LineOrigin> {
    match origin {
        ' ' => Some(LineOrigin::Context),
        '+' => Some(LineOrigin::Addition),
        '-' => Some(LineOrigin::Deletion),
        _ => None,
    }
}

/// Strips one trailing `\n` (or `\r\n`); libgit2 line/header buffers keep it.
fn trim_newline(text: &str) -> &str {
    let text = text.strip_suffix('\n').unwrap_or(text);
    text.strip_suffix('\r').unwrap_or(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Oid;

    /// Commits `files` on top of `parent` using in-memory trees (no worktree).
    fn commit_files(repo: &Repository, parent: Option<Oid>, files: &[(&str, &[u8])]) -> Oid {
        let mut builder = repo.treebuilder(None).unwrap();
        if let Some(parent) = parent {
            let parent_tree = repo.find_commit(parent).unwrap().tree_id();
            builder = repo
                .treebuilder(Some(&repo.find_tree(parent_tree).unwrap()))
                .unwrap();
        }
        for (name, content) in files {
            let blob = repo.blob(content).unwrap();
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

    fn fixture() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        (dir, repo)
    }

    #[test]
    fn diffstat_should_mark_root_commit_files_added() {
        let (_dir, repo) = fixture();
        let root = commit_files(&repo, None, &[("a.txt", b"one\n")]);
        let commit = repo.find_commit(root).unwrap();

        let stat = diffstat(&repo, &commit).unwrap();
        let file = &stat.files[0];
        assert_eq!(
            (
                stat.files_changed,
                file.path.as_str(),
                file.status,
                file.additions,
                file.deletions
            ),
            (1, "a.txt", DiffStatus::Added, 1, 0)
        );
    }

    #[test]
    fn diffstat_should_count_modified_lines() {
        let (_dir, repo) = fixture();
        let root = commit_files(&repo, None, &[("a.txt", b"one\ntwo\n")]);
        let child = commit_files(&repo, Some(root), &[("a.txt", b"one\nthree\nfour\n")]);
        let commit = repo.find_commit(child).unwrap();

        let stat = diffstat(&repo, &commit).unwrap();
        let file = &stat.files[0];
        assert_eq!(
            (file.status, file.additions, file.deletions, file.binary),
            (DiffStatus::Modified, 2, 1, false)
        );
    }

    #[test]
    fn diffstat_should_flag_binary_files_without_line_counts() {
        let (_dir, repo) = fixture();
        let root = commit_files(&repo, None, &[("blob.bin", b"\x00\x01\x02binary")]);
        let commit = repo.find_commit(root).unwrap();

        let stat = diffstat(&repo, &commit).unwrap();
        let file = &stat.files[0];
        assert_eq!((file.binary, file.additions, file.deletions), (true, 0, 0));
    }

    #[test]
    fn commit_diff_should_drop_whole_hunks_past_line_cap() {
        let (_dir, repo) = fixture();
        let big = (0..MAX_FILE_DIFF_LINES + 100)
            .map(|i| format!("line {i}\n"))
            .collect::<String>();
        let root = commit_files(&repo, None, &[("big.txt", big.as_bytes())]);
        let commit = repo.find_commit(root).unwrap();

        let diff = commit_diff(&repo, &commit, None).unwrap();
        let file = &diff.files[0];
        let rendered: usize = file.hunks.iter().map(|hunk| hunk.lines.len()).sum();
        // A single oversized hunk is dropped whole; the stats stay complete.
        assert_eq!(
            (file.truncated, rendered, file.stat.additions),
            (true, 0, MAX_FILE_DIFF_LINES + 100)
        );
    }

    #[test]
    fn commit_diff_should_keep_hunks_at_exact_line_cap() {
        let (_dir, repo) = fixture();
        // One hunk of exactly MAX_FILE_DIFF_LINES added lines.
        let exact = (0..MAX_FILE_DIFF_LINES)
            .map(|i| format!("line {i}\n"))
            .collect::<String>();
        let root = commit_files(&repo, None, &[("exact.txt", exact.as_bytes())]);
        let commit = repo.find_commit(root).unwrap();

        let diff = commit_diff(&repo, &commit, None).unwrap();
        let file = &diff.files[0];
        let rendered: usize = file.hunks.iter().map(|hunk| hunk.lines.len()).sum();
        assert_eq!((file.truncated, rendered), (false, MAX_FILE_DIFF_LINES));
    }

    #[test]
    fn commit_diff_should_report_line_numbers_and_origins() {
        let (_dir, repo) = fixture();
        let root = commit_files(&repo, None, &[("a.txt", b"one\ntwo\n")]);
        let child = commit_files(&repo, Some(root), &[("a.txt", b"one\nthree\n")]);
        let commit = repo.find_commit(child).unwrap();

        let diff = commit_diff(&repo, &commit, None).unwrap();
        let lines = &diff.files[0].hunks[0].lines;
        let shape: Vec<_> = lines
            .iter()
            .map(|line| {
                (
                    line.origin,
                    line.content.as_str(),
                    line.old_lineno,
                    line.new_lineno,
                )
            })
            .collect();
        assert_eq!(
            shape,
            vec![
                (LineOrigin::Context, "one", Some(1), Some(1)),
                (LineOrigin::Deletion, "two", Some(2), None),
                (LineOrigin::Addition, "three", None, Some(2)),
            ]
        );
    }
}
