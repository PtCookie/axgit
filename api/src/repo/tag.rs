//! Tag detail for `GET /api/v1/repos/{repo}/tags/{name}` (docs/API.md).
//!
//! Kept separate from `refs.rs` rather than folded into it: `TagDetail` is
//! not a superset of `TagRef` the way `CommitDetail` is of `CommitInfo` —
//! `TagRef.target` is the fully peeled commit sha (a link target), while this
//! module's `object` is the tag's one-level dereference (cgit's
//! `cgit_object_link()` input). The two endpoints keep `target` meaning the
//! same thing so a reader never has to hold two definitions in their head.

use git2::{ObjectType, Repository};
use serde::Serialize;
use utoipa::ToSchema;

use super::commits::{CommitAuthor, signature_info, time_rfc3339};
use crate::error::ApiError;

/// What kind of object a tag's one-level dereference points at. Deliberately
/// separate from `tree::EntryKind` — that enum's domain is tree entries (it
/// carries `symlink`, and its `commit` variant means gitlink), not tag
/// targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ObjectKind {
    Commit,
    Tree,
    Blob,
    Tag,
}

/// The object a tag points at, one dereference in. For an annotated tag this
/// is the tag object's own `object` header; for a lightweight tag it's the
/// ref's direct target. A nested tag (tag-of-tag) or a tag on a tree/blob
/// reports that object here, not the fully peeled commit — see
/// [`TagDetail::target`].
#[derive(Debug, Serialize, ToSchema)]
pub struct TagObject {
    pub sha: String,
    #[serde(rename = "type")]
    pub kind: ObjectKind,
}

/// Tag detail of `GET /api/v1/repos/{repo}/tags/{name}` (docs/API.md).
#[derive(Debug, Serialize, ToSchema)]
pub struct TagDetail {
    #[schema(example = "v1.0.0")]
    pub name: String,
    /// The annotated tag object's own sha. `None` for a lightweight tag —
    /// lightweight tags have no tag object, so `message`/`tagger`/`tagged_at`
    /// are `None` too and `object.sha == target`.
    #[schema(required = true)]
    pub tag_object: Option<String>,
    /// One dereference of the tag (see [`TagObject`]).
    pub object: TagObject,
    /// Fully peeled commit sha — the same value `GET /refs`' `tags[].target`
    /// reports. `None` when the tag chain never reaches a commit (a tag on a
    /// tree or blob), which is also exactly when an archive download is
    /// unavailable for this tag.
    #[schema(required = true)]
    pub target: Option<String>,
    /// Full tag message, trailing whitespace trimmed. `None` for a
    /// lightweight tag, a non-utf8 message, or an all-whitespace message.
    #[schema(required = true)]
    pub message: Option<String>,
    /// `None` for a lightweight tag, and for an annotated tag with no tagger
    /// line (git allows creating one).
    #[schema(required = true)]
    pub tagger: Option<CommitAuthor>,
    #[schema(required = true)]
    pub tagged_at: Option<String>,
}

/// Reads one tag's detail off `refs/tags/{name}`. Never `?`-propagates a
/// git2 error — like `repo::resolve::resolve_commit`, any lookup or peeling
/// failure becomes [`ApiError::RefNotFound`] so a corrupt or dangling ref
/// answers 404, not 500.
pub fn detail(repo: &Repository, name: &str) -> Result<TagDetail, ApiError> {
    let name = name.trim_matches('/');
    let not_found = || ApiError::RefNotFound(name.to_owned());
    let reference = repo
        .find_reference(&format!("refs/tags/{name}"))
        .map_err(|_| not_found())?;

    // `Some` only when the ref's target chain reaches a tag object — true for
    // every annotated tag, including a nested one (peeling an object to its
    // own type is a no-op, so this returns the outermost tag, not the
    // innermost).
    let tag = reference.peel_to_tag().ok();
    let object = match &tag {
        Some(tag) => {
            let kind = tag.target_type().ok_or_else(not_found)?;
            TagObject {
                sha: tag.target_id().to_string(),
                kind: object_kind(kind),
            }
        }
        None => {
            let direct = reference.peel(ObjectType::Any).map_err(|_| not_found())?;
            let kind = direct.kind().ok_or_else(not_found)?;
            TagObject {
                sha: direct.id().to_string(),
                kind: object_kind(kind),
            }
        }
    };

    let target = reference
        .peel_to_commit()
        .ok()
        .map(|commit| commit.id().to_string());

    let message = tag
        .as_ref()
        .and_then(|tag| tag.message())
        .map(str::trim_end)
        .filter(|message| !message.trim().is_empty())
        .map(str::to_owned);
    let tagger_signature = tag.as_ref().and_then(|tag| tag.tagger());
    let tagger = tagger_signature.as_ref().map(signature_info);
    let tagged_at = tagger_signature.and_then(|tagger| time_rfc3339(tagger.when()));

    Ok(TagDetail {
        name: name.to_owned(),
        tag_object: tag.as_ref().map(|tag| tag.id().to_string()),
        object,
        target,
        message,
        tagger,
        tagged_at,
    })
}

fn object_kind(kind: ObjectType) -> ObjectKind {
    match kind {
        ObjectType::Tree => ObjectKind::Tree,
        ObjectType::Blob => ObjectKind::Blob,
        ObjectType::Tag => ObjectKind::Tag,
        // `Commit` and the exhaustive-match placeholder (`Any`/`Invalid` are
        // never produced by a real peel) both fall back to `Commit` — the
        // overwhelmingly common case, and never reached for `Any`/`Invalid`
        // in practice since a peeled object always reports a concrete kind.
        _ => ObjectKind::Commit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_repo() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().expect("failed to create tempdir");
        let repo = Repository::init(dir.path()).expect("failed to init repo");
        (dir, repo)
    }

    /// Commits an empty tree, no worktree involved. Returns the commit id.
    fn commit_empty(repo: &Repository, message: &str) -> git2::Oid {
        let tree_oid = repo.treebuilder(None).unwrap().write().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let sig = git2::Signature::now("Test", "test@example.com").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[])
            .unwrap()
    }

    fn tagger_sig() -> git2::Signature<'static> {
        git2::Signature::new(
            "Tagger",
            "tagger@example.com",
            &git2::Time::new(1_753_000_000, 9 * 60),
        )
        .unwrap()
    }

    #[test]
    fn object_kind_should_map_every_git2_object_type() {
        assert_eq!(object_kind(ObjectType::Commit), ObjectKind::Commit);
        assert_eq!(object_kind(ObjectType::Tree), ObjectKind::Tree);
        assert_eq!(object_kind(ObjectType::Blob), ObjectKind::Blob);
        assert_eq!(object_kind(ObjectType::Tag), ObjectKind::Tag);
    }

    #[test]
    fn detail_on_a_lightweight_tag_pointing_at_a_blob_should_null_commit_fields() {
        let (_dir, repo) = fixture_repo();
        let blob = repo.blob(b"hello").unwrap();
        repo.reference("refs/tags/blob-tag", blob, false, "test")
            .unwrap();

        let detail = detail(&repo, "blob-tag").unwrap();
        assert_eq!(detail.tag_object, None);
        assert_eq!(detail.object.sha, blob.to_string());
        assert_eq!(detail.object.kind, ObjectKind::Blob);
        assert_eq!(detail.target, None, "a blob tag never reaches a commit");
        assert_eq!(detail.message, None);
        assert!(detail.tagger.is_none());
        assert_eq!(detail.tagged_at, None);
    }

    #[test]
    fn detail_on_an_annotated_tag_pointing_at_a_tree_should_null_target() {
        let (_dir, repo) = fixture_repo();
        let tree_oid = repo.treebuilder(None).unwrap().write().unwrap();
        let tree = repo.find_object(tree_oid, Some(ObjectType::Tree)).unwrap();
        let sig = tagger_sig();
        let tag_oid = repo
            .tag("tree-tag", &tree, &sig, "a tree tag", false)
            .unwrap();

        let detail = detail(&repo, "tree-tag").unwrap();
        assert_eq!(detail.tag_object, Some(tag_oid.to_string()));
        assert_eq!(detail.object.sha, tree_oid.to_string());
        assert_eq!(detail.object.kind, ObjectKind::Tree);
        assert_eq!(detail.target, None);
        assert_eq!(detail.message.as_deref(), Some("a tree tag"));
        assert_eq!(
            detail.tagger.as_ref().map(|a| a.name.as_str()),
            Some("Tagger")
        );
        assert!(detail.tagged_at.is_some());
    }

    #[test]
    fn detail_on_a_nested_tag_should_report_the_inner_tag_as_the_object() {
        let (_dir, repo) = fixture_repo();
        let commit = commit_empty(&repo, "initial");
        let commit_object = repo.find_object(commit, Some(ObjectType::Commit)).unwrap();
        let sig = tagger_sig();
        let inner_oid = repo
            .tag("inner", &commit_object, &sig, "inner tag", false)
            .unwrap();
        let inner_object = repo.find_object(inner_oid, Some(ObjectType::Tag)).unwrap();
        repo.tag("outer", &inner_object, &sig, "outer tag", false)
            .unwrap();

        let detail = detail(&repo, "outer").unwrap();
        // The outer tag object itself, not the inner one, is `tag_object` —
        // `peel_to_tag` on the ref stops at the first (outermost) tag.
        assert_eq!(detail.object.sha, inner_oid.to_string());
        assert_eq!(detail.object.kind, ObjectKind::Tag);
        assert_eq!(
            detail.target,
            Some(commit.to_string()),
            "target fully peels through both tags to the commit"
        );
        assert_eq!(detail.message.as_deref(), Some("outer tag"));
    }

    #[test]
    fn detail_should_trim_a_blank_message_to_none() {
        let (_dir, repo) = fixture_repo();
        let commit = commit_empty(&repo, "initial");
        let commit_object = repo.find_object(commit, Some(ObjectType::Commit)).unwrap();
        let sig = tagger_sig();
        repo.tag("blank", &commit_object, &sig, "   \n\t ", false)
            .unwrap();

        let detail = detail(&repo, "blank").unwrap();
        assert_eq!(detail.message, None);
        // A blank message doesn't null out the tagger — they're independent.
        assert!(detail.tagger.is_some());
    }

    #[test]
    fn detail_on_a_tag_object_with_no_tagger_line_should_null_tagger_and_tagged_at() {
        // `Repository::tag` always writes a tagger line (it requires a
        // `&Signature`); libgit2 itself tolerates one being absent
        // (`git_tag__parse_buffer`, tag.c), so the object is built by hand to
        // reach that branch of `detail()`.
        let (_dir, repo) = fixture_repo();
        let commit = commit_empty(&repo, "initial");
        let raw = format!("object {commit}\ntype commit\ntag no-tagger\n\nmessage body\n");
        let tag_oid = repo
            .odb()
            .unwrap()
            .write(ObjectType::Tag, raw.as_bytes())
            .unwrap();
        repo.reference("refs/tags/no-tagger", tag_oid, false, "test")
            .unwrap();

        let detail = detail(&repo, "no-tagger").unwrap();
        assert_eq!(detail.tag_object, Some(tag_oid.to_string()));
        assert_eq!(detail.object.kind, ObjectKind::Commit);
        assert_eq!(detail.target, Some(commit.to_string()));
        assert_eq!(detail.message.as_deref(), Some("message body"));
        assert!(detail.tagger.is_none());
        assert_eq!(detail.tagged_at, None);
    }

    #[test]
    fn detail_should_return_ref_not_found_for_an_unknown_tag() {
        let (_dir, repo) = fixture_repo();
        commit_empty(&repo, "initial");
        assert!(matches!(
            detail(&repo, "no-such-tag"),
            Err(ApiError::RefNotFound(name)) if name == "no-such-tag"
        ));
    }
}
