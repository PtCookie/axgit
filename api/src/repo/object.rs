//! Object-by-id lookups for `GET /api/v1/repos/{repo}/objects/{oid}` and
//! `.../objects/{oid}/raw` (docs/API.md).
//!
//! Every other route in this API resolves through a ref (branch/tag/sha)
//! plus, for trees/blobs, a path — there's no way to address an object by
//! its own id anywhere else. This module is the one exception, closing the
//! last "Tags and refs" cgit-parity gap (`cgit_object_link()`): a tag's
//! target (`repo::tag`) can now point somewhere clickable even when it
//! isn't a commit, and a tree entry (`repo::tree`) can link onward by its
//! own `sha` with no path context.

use git2::{ObjectType, Oid, Repository};
use serde::Serialize;
use utoipa::ToSchema;

use super::blob;
use super::commits::{CommitAuthor, signature_info, time_rfc3339};
use super::tag::{self, ObjectKind, TagObject};
use super::tree::{self, TreeEntryInfo};
use crate::error::ApiError;

/// Blob payload of the object endpoint. Same fields as `BlobInfo`
/// (`repo::blob`) minus `sha`/`path`/`mode` — an oid alone has neither a
/// path nor a mode; mode lives on the tree entry that references it.
#[derive(Debug, Serialize, ToSchema)]
pub struct ObjectBlob {
    /// Full size in bytes, regardless of `too_large`.
    pub size: u64,
    /// libgit2's NUL heuristic, or content that is not valid UTF-8.
    pub binary: bool,
    /// `true` past the 1 MiB inline-content cap; fetch `.../raw` instead.
    pub too_large: bool,
    /// UTF-8 content; `None` when `binary` or `too_large`.
    #[schema(required = true)]
    pub content: Option<String>,
}

/// Tree payload of the object endpoint. Entries link onward by their own
/// `sha` (`TreeEntryInfo::sha`) — there's no root commit here to build a
/// path from, so by-oid navigation is oid-to-oid rather than path-based.
#[derive(Debug, Serialize, ToSchema)]
pub struct ObjectTree {
    /// Trees first, then by name ascending — same ordering as `GET /tree`.
    pub entries: Vec<TreeEntryInfo>,
}

/// Tag payload of the object endpoint — a nested tag's own detail, same
/// shape and rules `GET /tags/{name}` reports, minus `name`/`tag_object`
/// (an oid alone has neither a ref name nor a need to restate its own sha,
/// already reported as the envelope's `sha`).
#[derive(Debug, Serialize, ToSchema)]
pub struct ObjectTag {
    /// One-level dereference of this tag (see `repo::tag::TagObject`).
    pub object: TagObject,
    /// Fully peeled commit sha. `None` when the chain never reaches one.
    #[schema(required = true)]
    pub target: Option<String>,
    /// Full tag message, trailing whitespace trimmed. `None` for a
    /// non-utf8 or all-whitespace message.
    #[schema(required = true)]
    pub message: Option<String>,
    /// `None` when the tag object has no tagger line (git allows creating
    /// one without).
    #[schema(required = true)]
    pub tagger: Option<CommitAuthor>,
    #[schema(required = true)]
    pub tagged_at: Option<String>,
}

/// Response of `GET /objects/{oid}` (docs/API.md). Every key is always
/// present (this document's general rule) rather than a `oneOf` union — one
/// envelope, with only the `type`-matching payload populated.
#[derive(Debug, Serialize, ToSchema)]
pub struct ObjectDetail {
    pub sha: String,
    #[serde(rename = "type")]
    pub kind: ObjectKind,
    #[schema(required = true)]
    pub tree: Option<ObjectTree>,
    #[schema(required = true)]
    pub blob: Option<ObjectBlob>,
    #[schema(required = true)]
    pub tag: Option<ObjectTag>,
}

/// Blob bytes for `GET /objects/{oid}/raw` (owned — handlers cross thread
/// bounds), the by-oid analogue of `repo::blob::RawBlob`.
pub struct RawObject {
    pub bytes: Vec<u8>,
    pub binary: bool,
}

/// Reads the object at `oid`, whatever kind it is. Unlike every other
/// lookup in this crate, a miss here is a genuinely missing object (no ref
/// or path resolution involved), so it maps to
/// [`ApiError::ObjectNotFound`], not [`ApiError::RefNotFound`].
pub fn read_object(repo: &Repository, oid: Oid) -> Result<ObjectDetail, ApiError> {
    let not_found = || ApiError::ObjectNotFound(oid.to_string());
    let object = repo.find_object(oid, None).map_err(|_| not_found())?;
    let sha = oid.to_string();

    match object.kind() {
        Some(ObjectType::Tree) => {
            let tree_obj = object.into_tree().map_err(|_| not_found())?;
            let entries = tree::entries_of(repo, &tree_obj)?;
            Ok(ObjectDetail {
                sha,
                kind: ObjectKind::Tree,
                tree: Some(ObjectTree { entries }),
                blob: None,
                tag: None,
            })
        }
        Some(ObjectType::Blob) => {
            let blob_obj = object.into_blob().map_err(|_| not_found())?;
            let (binary, too_large) = blob::classify(&blob_obj);
            let content = (!binary && !too_large).then(|| {
                // Safe: `classify` already confirmed this is valid utf8 when
                // not binary/too_large (same rule `blob::read_blob` relies on).
                String::from_utf8_lossy(blob_obj.content()).into_owned()
            });
            Ok(ObjectDetail {
                sha,
                kind: ObjectKind::Blob,
                tree: None,
                blob: Some(ObjectBlob {
                    size: blob_obj.content().len() as u64,
                    binary,
                    too_large,
                    content,
                }),
                tag: None,
            })
        }
        Some(ObjectType::Commit) => Ok(ObjectDetail {
            sha,
            kind: ObjectKind::Commit,
            tree: None,
            blob: None,
            tag: None,
        }),
        Some(ObjectType::Tag) => {
            let tag_obj = object.into_tag().map_err(|_| not_found())?;
            let (object, target) = tag::dereference_object(&tag_obj, not_found)?;
            let message = tag_obj
                .message()
                .map(str::trim_end)
                .filter(|message| !message.trim().is_empty())
                .map(str::to_owned);
            let tagger_signature = tag_obj.tagger();
            let tagger = tagger_signature.as_ref().map(signature_info);
            let tagged_at = tagger_signature.and_then(|tagger| time_rfc3339(tagger.when()));
            Ok(ObjectDetail {
                sha,
                kind: ObjectKind::Tag,
                tree: None,
                blob: None,
                tag: Some(ObjectTag {
                    object,
                    target,
                    message,
                    tagger,
                    tagged_at,
                }),
            })
        }
        // `find_object(oid, None)` always reports a concrete kind for a
        // successfully-resolved object; this arm exists only for
        // exhaustiveness.
        _ => Err(not_found()),
    }
}

/// Reads `oid` as a blob's raw bytes for `GET /objects/{oid}/raw`. Any
/// other kind (including a missing oid) is [`ApiError::ObjectNotFound`].
pub fn read_raw(repo: &Repository, oid: Oid) -> Result<RawObject, ApiError> {
    let not_found = || ApiError::ObjectNotFound(oid.to_string());
    let blob_obj = repo.find_blob(oid).map_err(|_| not_found())?;
    Ok(RawObject {
        bytes: blob_obj.content().to_vec(),
        binary: blob_obj.is_binary(),
    })
}

/// Parses `raw` as a **full**, lowercase-or-not 40-character hex oid —
/// `Oid::from_str` alone also accepts an abbreviation (any length up to 40),
/// which this endpoint deliberately rejects: every link axgit itself emits
/// carries a full oid, and requiring one is what lets every response here be
/// unconditionally immutable-cached (the address pins the content). Anything
/// else is `400 invalid_param`.
pub fn parse_full_oid(raw: &str) -> Result<Oid, ApiError> {
    let invalid =
        || ApiError::InvalidParam(format!("'{raw}' is not a full 40-character object id"));
    if raw.len() != 40 || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid());
    }
    Oid::from_str(raw).map_err(|_| invalid())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_oid_should_accept_a_full_lowercase_hex_oid() {
        let sha = "94739392266bcbd2a4dcc7e02f57b7cf4ba7ec02";
        assert_eq!(sha.len(), 40);
        assert_eq!(parse_full_oid(sha).unwrap(), Oid::from_str(sha).unwrap());
    }

    #[test]
    fn parse_full_oid_should_accept_uppercase_hex() {
        let sha = "94739392266BCBD2A4DCC7E02F57B7CF4BA7EC02";
        assert_eq!(sha.len(), 40);
        assert!(parse_full_oid(sha).is_ok());
    }

    #[test]
    fn parse_full_oid_should_reject_an_abbreviation() {
        assert!(matches!(
            parse_full_oid("9473939226"),
            Err(ApiError::InvalidParam(_))
        ));
    }

    #[test]
    fn parse_full_oid_should_reject_non_hex_characters() {
        let not_hex = "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz";
        assert_eq!(not_hex.len(), 40);
        assert!(matches!(
            parse_full_oid(not_hex),
            Err(ApiError::InvalidParam(_))
        ));
    }

    #[test]
    fn parse_full_oid_should_reject_a_too_long_string() {
        let too_long = "94739392266bcbd2a4dcc7e02f57b7cf4ba7ec021";
        assert_eq!(too_long.len(), 41);
        assert!(matches!(
            parse_full_oid(too_long),
            Err(ApiError::InvalidParam(_))
        ));
    }
}
