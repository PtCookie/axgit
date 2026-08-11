use std::path::{Path, PathBuf};

use git2::{Commit, Oid, Repository};
use serde::Serialize;
use sha2::{Digest, Sha256};
use utoipa::ToSchema;

use super::{diff, meta};
use crate::error::ApiError;

/// Commit author of `GET /api/v1/repos/{repo}/commits` (docs/API.md).
/// The raw email is never exposed; `email_hash` seeds locally generated avatars.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CommitAuthor {
    pub name: String,
    /// sha256 hex of the trimmed, lowercased author email.
    #[schema(example = "b642b4217b34b1e8d3bd915fc65c4452")]
    pub email_hash: String,
}

/// Log entry of `GET /api/v1/repos/{repo}/commits` (docs/API.md).
#[derive(Debug, Serialize, ToSchema)]
pub struct CommitInfo {
    /// Full commit sha.
    pub sha: String,
    /// First line of the commit message. `None` for non-utf8 commit messages.
    #[schema(required = true)]
    pub summary: Option<String>,
    /// Commit message past the summary line, trimmed. Present only when the
    /// request asked for it (`msg=1`); the key itself is omitted otherwise,
    /// for a commit with no body, and for a non-utf8 message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    pub author: CommitAuthor,
    /// Authordate (RFC 3339). `None` for unrepresentable timestamps.
    #[schema(required = true, example = "2026-07-01T14:00:00+09:00")]
    pub authored_at: Option<String>,
    pub parents: Vec<String>,
    /// The path filter's previous name, when this commit is the rename this
    /// entry was followed across (`follow=1`, docs/DECISIONS.md #56). Present
    /// only on the renaming commit itself, never on the commits before or
    /// after it — the key is omitted, not `null`, everywhere else (same rule
    /// as `body`, #44).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub renamed_from: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CommitsPage {
    pub commits: Vec<CommitInfo>,
    /// Opaque token for the next page; `null` on the last page. Pass it back
    /// verbatim as the `cursor` query parameter.
    #[schema(required = true)]
    pub next_cursor: Option<String>,
}

/// Opaque pagination token: a fixed walk start plus how many filtered commits
/// to skip before collecting `limit` more. Re-walking from a fixed `start`
/// (rather than resuming from the boundary commit alone) is what makes
/// pagination lossless — see `docs/DECISIONS.md` #37 for why a single-sha
/// cursor silently dropped side-branch commits pending at a page boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub start: Oid,
    pub offset: usize,
}

impl Cursor {
    /// Bounds the walk a manipulated cursor can force (same rationale as
    /// search/stats' scan budgets, `docs/DECISIONS.md` #26/#28).
    pub const MAX_OFFSET: usize = 100_000;

    /// Parses `"<full sha>.<offset>"`. Anything else — including the old
    /// bare-sha cursor format — is `None`, which callers turn into
    /// `400 invalid_param`.
    pub fn parse(raw: &str) -> Option<Self> {
        let (sha, offset) = raw.rsplit_once('.')?;
        let oid = Oid::from_str(sha).ok()?;
        // Round-trip check: rejects short/non-canonical hex that `Oid::from_str`
        // would otherwise zero-pad into a valid-looking Oid.
        if oid.to_string() != sha {
            return None;
        }
        let offset = offset.parse::<usize>().ok()?;
        if offset > Self::MAX_OFFSET {
            return None;
        }
        Some(Self { start: oid, offset })
    }

    pub fn encode(&self) -> String {
        format!("{}.{}", self.start, self.offset)
    }
}

/// Commit detail of `GET /api/v1/repos/{repo}/commits/{sha}` (docs/API.md).
/// Superset of [`CommitInfo`] so the frontend can extend the log entry type.
#[derive(Debug, Serialize, ToSchema)]
pub struct CommitDetail {
    /// Full commit sha, even when the request used a branch, tag or short sha.
    pub sha: String,
    #[schema(required = true)]
    pub summary: Option<String>,
    /// Full commit message. `None` for non-utf8 messages.
    #[schema(required = true)]
    pub message: Option<String>,
    /// `git notes` message attached to this commit on the repository's
    /// default notes ref (`refs/notes/commits`, or `core.notesRef` when set).
    /// `None` when there is no note, no notes ref at all, or the note is
    /// non-utf8 or blank. Only the default ref is read — cgit's
    /// `notes.displayRef`/multi-ref concatenation has no analogue here.
    #[schema(required = true)]
    pub note: Option<String>,
    pub author: CommitAuthor,
    pub committer: CommitAuthor,
    #[schema(required = true)]
    pub authored_at: Option<String>,
    #[schema(required = true)]
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
        note: note(repo, commit),
        author: signature_info(&commit.author()),
        committer: signature_info(&commit.committer()),
        authored_at: time_rfc3339(commit.author().when()),
        committed_at: time_rfc3339(commit.committer().when()),
        parents: commit.parent_ids().map(|id| id.to_string()).collect(),
        diffstat: diff::diffstat(repo, commit)?,
    })
}

/// Reads this commit's note off the repository's default notes ref. Any
/// lookup failure — no notes ref, no note for this commit, a non-utf8
/// message — is `None`, never an error: a repository with no notes at all
/// (every fixture git-compose produces) must not turn a working commit page
/// into a 500. A note that is present but all-whitespace also collapses to
/// `None`, same treatment as an empty commit message.
fn note(repo: &Repository, commit: &Commit) -> Option<String> {
    let note = repo.find_note(None, commit.id()).ok()?;
    let message = note.message()?.trim_end();
    (!message.trim().is_empty()).then(|| message.to_owned())
}

/// Parameters for [`log`] — grouped once `follow` joined `path`/`skip`/
/// `limit`/`include_body`, the same "one params struct" shape as
/// `diff::DiffParams`.
pub struct LogParams<'a> {
    pub path: Option<&'a Path>,
    pub skip: usize,
    pub limit: usize,
    pub include_body: bool,
    /// Follow the `path` filter across whole-file renames (`follow=1`,
    /// docs/DECISIONS.md #56). Ignored when `path` is `None` — there is
    /// nothing to follow.
    pub follow: bool,
}

/// Bounds how many rename lookups (each a full first-parent tree diff via
/// `diff::rename_source`) a single `follow=1` walk performs — same
/// rationale as `Cursor::MAX_OFFSET` and search/stats' scan budgets
/// (`docs/DECISIONS.md` #26/#28). Once hit, the walk keeps filtering on
/// whichever path it was last tracking rather than erroring.
const MAX_FOLLOW_RENAME_LOOKUPS: usize = 100;

/// Walks history from `start` (inclusive), skips the first `skip` commits
/// that pass the `path` filter, then collects up to `limit` more. libgit2's
/// default walk order is the closest match to `git log`, so no explicit
/// sorting is set.
///
/// Re-walking from the same fixed `start` on every page (instead of resuming
/// from the boundary commit alone) is deliberate: it's what makes pagination
/// lossless across side branches (`docs/DECISIONS.md` #37) — and, with
/// `follow`, what keeps the tracked path's rename history identical on every
/// page, since it's re-derived from `start` each time rather than resumed.
pub fn log(repo: &Repository, start: Oid, params: &LogParams<'_>) -> Result<CommitsPage, ApiError> {
    let &LogParams {
        path,
        skip,
        limit,
        include_body,
        follow,
    } = params;
    let mut revwalk = repo.revwalk()?;
    revwalk.push(start)?;
    let mut commits = Vec::new();
    let mut skipped = 0usize;
    let mut has_more = false;
    // The path currently being filtered on; mutates mid-walk when `follow`
    // crosses a rename.
    let mut tracked = path.map(Path::to_path_buf);
    let mut follow_lookups = 0usize;
    for oid in revwalk {
        let oid = oid?;
        let commit = repo.find_commit(oid)?;
        let mut renamed_from = None;
        if let Some(path) = tracked.as_deref() {
            if !touches_path(&commit, path) {
                continue;
            }
            // A rename looks like this from `path`'s side: the entry is
            // present in `commit` but absent from its first parent. Only
            // attempt the (relatively expensive) full tree diff when that
            // shape holds, so an ordinary modify/add never pays for it.
            if follow
                && follow_lookups < MAX_FOLLOW_RENAME_LOOKUPS
                && path_entry_id(&commit, path).is_some()
                && first_parent_lacks_path(&commit, path)
            {
                follow_lookups += 1;
                if let Some(old) = diff::rename_source(repo, &commit, path)? {
                    tracked = Some(PathBuf::from(&old));
                    renamed_from = Some(old);
                }
            }
        }
        if skipped < skip {
            skipped += 1;
            continue;
        }
        if commits.len() == limit {
            has_more = true;
            break;
        }
        let mut info = if include_body {
            commit_info_with_body(&commit)
        } else {
            commit_info(&commit)
        };
        info.renamed_from = renamed_from;
        commits.push(info);
    }
    let next_cursor = has_more.then(|| {
        Cursor {
            start,
            offset: skip + limit,
        }
        .encode()
    });
    Ok(CommitsPage {
        commits,
        next_cursor,
    })
}

/// Shared by the log walk and search's commit-message matcher
/// (`repo/search.rs`). Never carries `body` — see [`commit_info_with_body`].
pub(crate) fn commit_info(commit: &Commit) -> CommitInfo {
    CommitInfo {
        sha: commit.id().to_string(),
        summary: commit.summary().map(str::to_owned),
        body: None,
        author: signature_info(&commit.author()),
        authored_at: time_rfc3339(commit.author().when()),
        parents: commit.parent_ids().map(|id| id.to_string()).collect(),
        renamed_from: None,
    }
}

/// [`commit_info`] plus the message body (`?msg=1` on `GET /commits`).
pub(crate) fn commit_info_with_body(commit: &Commit) -> CommitInfo {
    CommitInfo {
        body: commit.body().map(str::to_owned),
        ..commit_info(commit)
    }
}

pub(crate) fn signature_info(signature: &git2::Signature) -> CommitAuthor {
    CommitAuthor {
        name: String::from_utf8_lossy(signature.name_bytes()).into_owned(),
        email_hash: email_hash(signature.email_bytes()),
    }
}

pub(crate) fn time_rfc3339(time: git2::Time) -> Option<String> {
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

/// Whether `commit`'s first parent has no entry at `path` — the shape a
/// rename takes on `path`'s side (the old path vanishes, the new one
/// appears). `true` for a root commit (no first parent to check), which is
/// harmless: [`diff::rename_source`] short-circuits to `None` for those too.
fn first_parent_lacks_path(commit: &Commit, path: &Path) -> bool {
    match commit.parent(0) {
        Ok(parent) => path_entry_id(&parent, path).is_none(),
        Err(_) => true,
    }
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
    fn cursor_round_trips_through_encode_and_parse() {
        let cursor = Cursor {
            start: Oid::from_str("0123456789abcdef0123456789abcdef01234567").unwrap(),
            offset: 42,
        };
        assert_eq!(Cursor::parse(&cursor.encode()), Some(cursor));
    }

    #[test]
    fn cursor_parse_rejects_malformed_input() {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        // The old bare-sha cursor format is no longer accepted.
        assert_eq!(Cursor::parse(sha), None);
        assert_eq!(Cursor::parse(&format!("{sha}.abc")), None);
        assert_eq!(Cursor::parse(&format!("{sha}.-1")), None);
        assert_eq!(
            Cursor::parse(&format!("{sha}.{}", Cursor::MAX_OFFSET + 1)),
            None
        );
        assert!(Cursor::parse(&format!("{sha}.{}", Cursor::MAX_OFFSET)).is_some());
        assert_eq!(Cursor::parse("zzz.0"), None);
        // Short hex must not zero-pad into a valid Oid.
        assert_eq!(Cursor::parse("0123.0"), None);
    }

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

    #[test]
    fn log_should_follow_renames_only_when_requested() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let root = commit_files(&repo, None, &[("a.txt", "one\ntwo\n")]);
        // Same content under a new name: a pure rename from `find_similar`'s
        // perspective, same trick `repo::blame`'s rename test uses.
        let renamed = commit_files(&repo, Some(root), &[("b.txt", "one\ntwo\n")]);
        let child = commit_files(&repo, Some(renamed), &[("b.txt", "one\nTHREE\n")]);

        let params = LogParams {
            path: Some(Path::new("b.txt")),
            skip: 0,
            limit: 10,
            include_body: false,
            follow: true,
        };
        let page = log(&repo, child, &params).unwrap();
        let shas: Vec<_> = page.commits.iter().map(|c| c.sha.as_str()).collect();
        assert_eq!(
            shas,
            [child.to_string(), renamed.to_string(), root.to_string()]
        );
        assert!(page.commits[0].renamed_from.is_none());
        assert_eq!(page.commits[1].renamed_from.as_deref(), Some("a.txt"));
        assert!(page.commits[2].renamed_from.is_none());

        // Without `follow`, the walk stops at the renaming commit.
        let no_follow = LogParams {
            follow: false,
            ..params
        };
        let page = log(&repo, child, &no_follow).unwrap();
        let shas: Vec<_> = page.commits.iter().map(|c| c.sha.as_str()).collect();
        assert_eq!(shas, [child.to_string(), renamed.to_string()]);
        assert!(page.commits.iter().all(|c| c.renamed_from.is_none()));
    }
}
