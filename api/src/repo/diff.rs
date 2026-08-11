//! Diffstat and structured diff for a single commit (docs/API.md).
//!
//! All diffs are first-parent: a merge shows its changes against parent 0 and
//! the root commit diffs against the empty tree. Rename detection runs with
//! libgit2 defaults on both entry points so the diffstat and the diff always
//! agree on the file list.

use std::path::Path;

use git2::{
    Commit, Delta, Diff, DiffDelta, DiffFormat, DiffOptions, Email, EmailCreateOptions, Patch,
    Repository, Sort, Tree,
};
use serde::Serialize;
use utoipa::ToSchema;

use crate::error::ApiError;

/// Per-file cap on rendered diff lines. Hunks are dropped whole once the cap
/// would be crossed — a hunk is never cut in the middle. Applies to the
/// structured JSON diffs only: the raw patch/rawdiff endpoints render in full.
pub const MAX_FILE_DIFF_LINES: usize = 1000;
/// Cap on files rendered by the diff endpoints (JSON only, see above). The
/// diffstat has no cap, so the full file list stays available on the commit
/// detail response.
pub const MAX_DIFF_FILES: usize = 300;
/// libgit2's own default, and what plain `git diff` uses.
pub const DEFAULT_CONTEXT: u32 = 3;
/// Upper bound on the `context=` query param. Keeps the cache-key value space
/// finite and stops a huge value from rendering whole files as one hunk.
pub const MAX_CONTEXT: u32 = 100;
/// Upper bound on the number of commits `format_patch` will render as one
/// series. cgit has no analogue (no `patch` endpoint at all); a half-applied,
/// silently-truncated patch series is worse than an error, so this rejects
/// rather than truncates.
pub const MAX_PATCH_COMMITS: usize = 100;

/// Display options every diff entry point threads through to `DiffOptions`.
/// `Copy` so handlers can move it into the `cached_response` closure freely.
#[derive(Debug, Clone, Copy)]
pub struct DiffParams<'a> {
    /// Literal single-path restriction; glob matching is not part of the contract.
    pub path: Option<&'a Path>,
    pub context: u32,
    /// cgit `ignorews=1` → libgit2 `GIT_DIFF_IGNORE_WHITESPACE`
    /// (`git diff --ignore-all-space`). Line `content` is still the original
    /// bytes — libgit2 only ignores whitespace when deciding what changed.
    pub ignore_whitespace: bool,
}

impl Default for DiffParams<'_> {
    fn default() -> Self {
        Self {
            path: None,
            context: DEFAULT_CONTEXT,
            ignore_whitespace: false,
        }
    }
}

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

/// Two-revision diff of `GET /api/v1/repos/{repo}/diff` (docs/API.md). A
/// plain tree-to-tree comparison (`git diff <from> <to>`), not a merge-base
/// `...` diff.
#[derive(Debug, Serialize, ToSchema)]
pub struct RevDiff {
    /// Resolved full sha of the old side. `null` only when `from` was omitted
    /// and `to` is a root commit (the old side is then the empty tree).
    #[schema(required = true)]
    pub from: Option<String>,
    /// Resolved full sha of the new side.
    pub to: String,
    /// `true` when files beyond [`MAX_DIFF_FILES`] were omitted. Always
    /// `false` in stat-only mode (`?stat=1`) — there is nothing to cap when
    /// `files` itself is always empty.
    pub truncated: bool,
    /// The full, uncapped file list — same rule as the commit diffstat.
    pub diffstat: DiffStat,
    /// Always empty in stat-only mode (`?stat=1`, [`rev_diff_stat`]) — the
    /// caller only wanted `diffstat`, so hunks are never rendered.
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

/// Diffstat against the first parent, covering every changed file. Always
/// uses default display options — the diffstat is the canonical file list for
/// a commit (docs/API.md) and must not vary with a display toggle.
pub fn diffstat(repo: &Repository, commit: &Commit) -> Result<DiffStat, ApiError> {
    let (old_tree, new_tree) = commit_trees(commit)?;
    diffstat_for_trees(repo, old_tree.as_ref(), Some(&new_tree))
}

/// File/line counts for the commit log's `stat=1` column (cgit's
/// `enable-log-filecount`/`enable-log-linecount`, docs/DECISIONS.md #57).
#[derive(Debug, Serialize, ToSchema)]
pub struct StatCounts {
    pub files_changed: usize,
    pub additions: usize,
    pub deletions: usize,
}

/// First-parent stat counts, restricted to `path` when given (the log's own
/// `path` filter, or its `follow`-tracked equivalent at the time of this
/// commit). A single libgit2 `Diff::stats()` call — unlike [`diffstat`], this
/// never loads a per-file `Patch` (no rename/status detail needed), which is
/// what keeps it cheap enough to run once per log row.
pub fn stat_counts(
    repo: &Repository,
    commit: &Commit,
    path: Option<&Path>,
) -> Result<StatCounts, ApiError> {
    let (old_tree, new_tree) = commit_trees(commit)?;
    let params = DiffParams {
        path,
        ..DiffParams::default()
    };
    let diff = build_diff(repo, old_tree.as_ref(), Some(&new_tree), &params)?;
    let stats = diff.stats()?;
    Ok(StatCounts {
        files_changed: stats.files_changed(),
        additions: stats.insertions(),
        deletions: stats.deletions(),
    })
}

/// Structured diff against the first parent, honoring `params`. A `path` the
/// commit's changes don't touch yields an empty file list, not an error —
/// same contract as the log endpoint's `path` filter.
pub fn commit_diff(
    repo: &Repository,
    commit: &Commit,
    params: &DiffParams<'_>,
) -> Result<CommitDiff, ApiError> {
    let (old_tree, new_tree) = commit_trees(commit)?;
    let diff = build_diff(repo, old_tree.as_ref(), Some(&new_tree), params)?;
    let (files, truncated) = render_files(&diff)?;
    Ok(CommitDiff {
        sha: commit.id().to_string(),
        parent: commit.parent_id(0).ok().map(|id| id.to_string()),
        truncated,
        files,
    })
}

/// Two-revision diff honoring `params`. `from: None` means `to`'s first
/// parent (the empty tree for a root commit) — so
/// `rev_diff(repo, None, &c, p)` and `commit_diff(repo, &c, p)` produce
/// identical `files`, which is asserted in the integration tests.
pub fn rev_diff(
    repo: &Repository,
    from: Option<&Commit>,
    to: &Commit,
    params: &DiffParams<'_>,
) -> Result<RevDiff, ApiError> {
    let (old_tree, from_sha, new_tree) = rev_sides(from, to)?;
    let diff = build_diff(repo, old_tree.as_ref(), Some(&new_tree), params)?;
    let (files, truncated) = render_files(&diff)?;
    let diffstat = diffstat_for_trees(repo, old_tree.as_ref(), Some(&new_tree))?;
    Ok(RevDiff {
        from: from_sha,
        to: to.id().to_string(),
        truncated,
        diffstat,
        files,
    })
}

/// Same comparison as [`rev_diff`], but skips the hunk render entirely
/// (`?stat=1` — cgit's `dt=2`). `files` is always empty and `truncated` is
/// always `false`: [`MAX_DIFF_FILES`] is a cap on rendered hunks, and there
/// are none to cap here. `diffstat` is uncapped either way, so this is
/// strictly cheaper than `rev_diff`, never smaller in what it reports.
/// `params.context`/`params.ignore_whitespace` are accepted for a uniform
/// call shape but have no effect — `diffstat_for_trees` always uses
/// [`DiffParams::default`].
pub fn rev_diff_stat(
    repo: &Repository,
    from: Option<&Commit>,
    to: &Commit,
    _params: &DiffParams<'_>,
) -> Result<RevDiff, ApiError> {
    let (old_tree, from_sha, new_tree) = rev_sides(from, to)?;
    let diffstat = diffstat_for_trees(repo, old_tree.as_ref(), Some(&new_tree))?;
    Ok(RevDiff {
        from: from_sha,
        to: to.id().to_string(),
        truncated: false,
        diffstat,
        files: Vec::new(),
    })
}

/// The (old tree, old sha, new tree) triple a two-revision comparison diffs
/// between — shared by [`rev_diff`] and [`rev_diff_stat`]. `from: None` means
/// `to`'s first parent (the empty tree for a root commit).
fn rev_sides<'r>(
    from: Option<&Commit<'r>>,
    to: &Commit<'r>,
) -> Result<(Option<Tree<'r>>, Option<String>, Tree<'r>), ApiError> {
    let (old_tree, from_sha) = match from {
        Some(commit) => (Some(commit.tree()?), Some(commit.id().to_string())),
        None => (
            first_parent_tree(to)?,
            to.parent_id(0).ok().map(|id| id.to_string()),
        ),
    };
    let new_tree = to.tree()?;
    Ok((old_tree, from_sha, new_tree))
}

/// Plain unified diff between two revisions (`cmd=rawdiff`), honoring
/// `params`. **No caps**: unlike the JSON diffs, a truncated patch would be a
/// corrupt one, defeating the endpoint's purpose (`git apply`/`git am`).
pub fn raw_diff(
    repo: &Repository,
    from: Option<&Commit>,
    to: &Commit,
    params: &DiffParams<'_>,
) -> Result<Vec<u8>, ApiError> {
    let old_tree = match from {
        Some(commit) => Some(commit.tree()?),
        None => first_parent_tree(to)?,
    };
    let new_tree = to.tree()?;
    let diff = build_diff(repo, old_tree.as_ref(), Some(&new_tree), params)?;
    let mut out = Vec::new();
    diff.print(DiffFormat::Patch, |_delta, _hunk, line| {
        // 'F' (file header) and 'H' (hunk header) lines already carry their
        // full text; only the three content origins need the prefix
        // char libgit2 deliberately excludes from `content()` re-added.
        if matches!(line.origin(), ' ' | '+' | '-') {
            out.push(line.origin() as u8);
        }
        out.extend_from_slice(line.content());
        true
    })?;
    Ok(out)
}

/// `git format-patch`-style mbox series for the commit range `(from, to]`
/// (`cmd=patch`) — `from` is *excluded*, unlike `raw_diff`'s `from`, which is
/// the other side of a tree comparison. `from: None` renders a single patch
/// for `to` alone. Every commit is diffed against its own first parent
/// (including merges), consistent with the rest of this module rather than
/// `git format-patch`'s default of skipping merges entirely.
///
/// Deliberately **not capped or truncated** past [`MAX_PATCH_COMMITS`]: a
/// silently-shortened series would apply cleanly and corrupt history, so an
/// oversized range is rejected outright.
pub fn format_patch(
    repo: &Repository,
    from: Option<&Commit>,
    to: &Commit,
    path: Option<&Path>,
) -> Result<Vec<u8>, ApiError> {
    // `from: None` is a single patch for `to` alone, not "walk to the root" —
    // there is nothing to hide, so a revwalk isn't the right tool at all.
    let oids = match from {
        None => vec![to.id()],
        Some(from) => {
            let mut revwalk = repo.revwalk()?;
            // Topological (children before parents) with time as a tiebreak
            // — default `Sort::NONE` is documented as implementation-defined,
            // and pure time sorting is unreliable when commits share a
            // timestamp (common in fixtures, possible in real history from a
            // fast rebase/import). Reversed below to the oldest -> newest
            // series order `git format-patch` produces.
            revwalk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME)?;
            revwalk.push(to.id())?;
            revwalk.hide(from.id())?;
            let mut oids = Vec::new();
            for oid in revwalk {
                oids.push(oid?);
                if oids.len() > MAX_PATCH_COMMITS {
                    return Err(ApiError::InvalidParam(format!(
                        "commit range exceeds the {MAX_PATCH_COMMITS}-commit patch limit"
                    )));
                }
            }
            oids.reverse();
            oids
        }
    };

    let total = oids.len();
    let mut out = Vec::new();
    let diff_params = DiffParams {
        path,
        ..DiffParams::default()
    };
    for (idx, oid) in oids.iter().enumerate() {
        let commit = repo.find_commit(*oid)?;
        let old_tree = first_parent_tree(&commit)?;
        let new_tree = commit.tree()?;
        let diff = build_diff(repo, old_tree.as_ref(), Some(&new_tree), &diff_params)?;
        let commit_id = commit.id();
        let summary = commit.summary_bytes().unwrap_or_default();
        let body = commit.body_bytes().unwrap_or_default();
        let email = Email::from_diff(
            &diff,
            idx + 1,
            total,
            &commit_id,
            summary,
            body,
            &commit.author(),
            &mut EmailCreateOptions::new(),
        )?;
        out.extend_from_slice(email.as_slice());
    }
    Ok(out)
}

/// The (old, new) tree pair a commit diffs against — its first parent, or the
/// empty tree (`None`) for a root commit.
fn commit_trees<'r>(commit: &Commit<'r>) -> Result<(Option<Tree<'r>>, Tree<'r>), ApiError> {
    Ok((first_parent_tree(commit)?, commit.tree()?))
}

/// A commit's first-parent tree, or `None` for a root commit (the empty tree).
fn first_parent_tree<'r>(commit: &Commit<'r>) -> Result<Option<Tree<'r>>, ApiError> {
    if commit.parent_count() > 0 {
        Ok(Some(commit.parent(0)?.tree()?))
    } else {
        Ok(None)
    }
}

/// Uncapped diffstat between two trees, always with default display options
/// — shared by the commit diffstat and the two-revision diff's `diffstat`
/// field, neither of which should vary with a hunk-display toggle.
fn diffstat_for_trees(
    repo: &Repository,
    old_tree: Option<&Tree<'_>>,
    new_tree: Option<&Tree<'_>>,
) -> Result<DiffStat, ApiError> {
    let diff = build_diff(repo, old_tree, new_tree, &DiffParams::default())?;
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

/// Tree-to-tree diff with rename detection and `params`' display options
/// applied. `None` on either side is the empty tree.
fn build_diff<'r>(
    repo: &'r Repository,
    old_tree: Option<&Tree<'_>>,
    new_tree: Option<&Tree<'_>>,
    params: &DiffParams<'_>,
) -> Result<Diff<'r>, ApiError> {
    let mut opts = DiffOptions::new();
    if let Some(path) = params.path {
        // Literal single-path restriction; glob matching is not part of the
        // API contract.
        opts.pathspec(path);
        opts.disable_pathspec_match(true);
    }
    opts.context_lines(params.context);
    if params.ignore_whitespace {
        opts.ignore_whitespace(true);
    }
    let mut diff = repo.diff_tree_to_tree(old_tree, new_tree, Some(&mut opts))?;
    // libgit2 defaults: renames only (no copies), 50% similarity.
    diff.find_similar(None)?;
    Ok(diff)
}

/// The path `new_path` had in `commit`'s first parent, when `commit` is the
/// one that renamed it there — `git log --follow`/cgit `enable-follow-links`
/// parity for the commit log's path filter (`docs/DECISIONS.md` #56). `None`
/// for a root commit, or when `commit` is not a pure rename of `new_path`.
///
/// Deliberately **not** pathspec-restricted like [`build_diff`]'s other
/// callers: a rename's old path can't be predicted, so the full first-parent
/// tree is diffed. Copies are not followed — only `Delta::Renamed` — matching
/// `repo::blame`'s "only whole-file renames are tracked" rule so the two
/// history-following features stay consistent with each other.
pub(crate) fn rename_source(
    repo: &Repository,
    commit: &Commit,
    new_path: &Path,
) -> Result<Option<String>, ApiError> {
    if commit.parent_count() == 0 {
        return Ok(None);
    }
    let old_tree = commit.parent(0)?.tree()?;
    let new_tree = commit.tree()?;
    let mut diff = repo.diff_tree_to_tree(Some(&old_tree), Some(&new_tree), None)?;
    diff.find_similar(None)?;
    for idx in 0..diff.deltas().len() {
        let Some(delta) = diff.get_delta(idx) else {
            continue;
        };
        if delta.status() != Delta::Renamed || delta.new_file().path() != Some(new_path) {
            continue;
        }
        return Ok(delta
            .old_file()
            .path()
            .map(|p| p.to_string_lossy().into_owned()));
    }
    Ok(None)
}

/// Runs the [`MAX_DIFF_FILES`]/[`MAX_FILE_DIFF_LINES`] render loop shared by
/// every JSON diff entry point. Returns `(files, truncated)`.
fn render_files(diff: &Diff<'_>) -> Result<(Vec<FileDiff>, bool), ApiError> {
    let delta_count = diff.deltas().len();
    let rendered = delta_count.min(MAX_DIFF_FILES);
    let mut files = Vec::with_capacity(rendered);
    for idx in 0..rendered {
        let (patch, stat) = load_file(diff, idx)?;
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
    Ok((files, delta_count > MAX_DIFF_FILES))
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

        let diff = commit_diff(&repo, &commit, &DiffParams::default()).unwrap();
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

        let diff = commit_diff(&repo, &commit, &DiffParams::default()).unwrap();
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

        let diff = commit_diff(&repo, &commit, &DiffParams::default()).unwrap();
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

    #[test]
    fn commit_diff_should_widen_hunks_with_more_context() {
        let (_dir, repo) = fixture();
        // Two changes 8 lines apart: default context (3) keeps them as two
        // separate hunks; context=10 pulls them into a shared hunk.
        let mut lines: Vec<String> = (0..20).map(|i| format!("line {i}\n")).collect();
        let root = commit_files(&repo, None, &[("a.txt", lines.join("").as_bytes())]);
        lines[2] = "line 2 changed\n".to_owned();
        lines[11] = "line 11 changed\n".to_owned();
        let child = commit_files(&repo, Some(root), &[("a.txt", lines.join("").as_bytes())]);
        let commit = repo.find_commit(child).unwrap();

        let default_diff = commit_diff(&repo, &commit, &DiffParams::default()).unwrap();
        assert_eq!(default_diff.files[0].hunks.len(), 2);

        let wide_diff = commit_diff(
            &repo,
            &commit,
            &DiffParams {
                context: 10,
                ..DiffParams::default()
            },
        )
        .unwrap();
        assert_eq!(wide_diff.files[0].hunks.len(), 1);
    }

    #[test]
    fn commit_diff_should_ignore_whitespace_only_changes_when_requested() {
        let (_dir, repo) = fixture();
        let root = commit_files(&repo, None, &[("a.txt", b"one\ntwo\n")]);
        // Only whitespace changed on the second line.
        let child = commit_files(&repo, Some(root), &[("a.txt", b"one\ntwo  \n")]);
        let commit = repo.find_commit(child).unwrap();

        let default_diff = commit_diff(&repo, &commit, &DiffParams::default()).unwrap();
        assert!(
            !default_diff.files[0].hunks.is_empty(),
            "expected the whitespace change to be visible by default: {default_diff:?}"
        );

        let ignorews_diff = commit_diff(
            &repo,
            &commit,
            &DiffParams {
                ignore_whitespace: true,
                ..DiffParams::default()
            },
        )
        .unwrap();
        // libgit2 still reports the file as changed (it went through
        // find_similar/Modified), but with no visible hunks once whitespace
        // is ignored.
        // libgit2 recomputes line stats from the whitespace-ignoring patch
        // too, so additions/deletions collapse to 0 along with the hunks.
        assert_eq!(
            (
                ignorews_diff.files.len(),
                ignorews_diff.files[0].hunks.len(),
                ignorews_diff.files[0].stat.additions,
                ignorews_diff.files[0].stat.deletions,
            ),
            (1, 0, 0, 0),
            "unexpected diff with ignore_whitespace: {ignorews_diff:?}"
        );
    }
}
