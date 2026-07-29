//! Handlers for the file-browsing endpoints: tree, blob, raw, and readme.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use super::{IMMUTABLE_CACHE_CONTROL, sha_addressed_json};
use crate::error::ApiError;
use crate::repo::{blob, open, readme, resolve, tree};
use crate::state::AppState;

/// `GET /api/v1/repos/{repo}/tree/{ref}/{path...}`
pub async fn get_tree(
    State(state): State<AppState>,
    Path((name, rest)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let root = state.config.repo_root.clone();
    let (immutable, listing) = tokio::task::spawn_blocking(move || {
        let repo = open::open_named(&root, &name)?;
        let split = resolve::resolve_ref_path(&repo, &rest)?;
        let listing = tree::list_tree(&repo, &split.commit, &split.path)?;
        Ok::<_, ApiError>((split.refname == listing.sha, listing))
    })
    .await
    .map_err(|err| ApiError::Internal(err.into()))??;
    Ok(sha_addressed_json(immutable, listing))
}

/// `GET /api/v1/repos/{repo}/blob/{ref}/{path...}`
pub async fn get_blob(
    State(state): State<AppState>,
    Path((name, rest)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let root = state.config.repo_root.clone();
    let (immutable, info) = tokio::task::spawn_blocking(move || {
        let repo = open::open_named(&root, &name)?;
        let split = resolve::resolve_ref_path(&repo, &rest)?;
        let info = blob::read_blob(&repo, &split.commit, &split.path)?;
        Ok::<_, ApiError>((split.refname == info.sha, info))
    })
    .await
    .map_err(|err| ApiError::Internal(err.into()))??;
    Ok(sha_addressed_json(immutable, info))
}

/// `GET /api/v1/repos/{repo}/raw/{ref}/{path...}`
pub async fn get_raw(
    State(state): State<AppState>,
    Path((name, rest)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let root = state.config.repo_root.clone();
    let (immutable, content_type, bytes) = tokio::task::spawn_blocking(move || {
        let repo = open::open_named(&root, &name)?;
        let split = resolve::resolve_ref_path(&repo, &rest)?;
        let raw = blob::read_raw(&repo, &split.commit, &split.path)?;
        // Extension-based detection; the content-derived fallback matters for
        // extensionless files (LICENSE, Makefile, ...).
        let content_type = mime_guess::from_path(&split.path).first_raw().unwrap_or({
            if raw.binary {
                "application/octet-stream"
            } else {
                "text/plain; charset=utf-8"
            }
        });
        let immutable = split.refname == split.commit.id().to_string();
        Ok::<_, ApiError>((immutable, content_type, raw.bytes))
    })
    .await
    .map_err(|err| ApiError::Internal(err.into()))??;

    let mut response = (
        [
            (header::CONTENT_TYPE, content_type),
            // Repository contents are untrusted input; never let the browser sniff.
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        bytes,
    )
        .into_response();
    if immutable {
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static(IMMUTABLE_CACHE_CONTROL),
        );
    }
    Ok(response)
}

#[derive(Deserialize)]
pub struct ReadmeQuery {
    /// Branch, tag, or commit sha; HEAD when absent.
    #[serde(rename = "ref")]
    r#ref: Option<String>,
}

/// `GET /api/v1/repos/{repo}/readme?ref=`
pub async fn get_readme(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<ReadmeQuery>,
) -> Result<Response, ApiError> {
    let root = state.config.repo_root.clone();
    let (immutable, info) = tokio::task::spawn_blocking(move || {
        let repo = open::open_named(&root, &name)?;
        let refname = query.r#ref.as_deref().unwrap_or("HEAD");
        let commit = resolve::resolve_commit(&repo, refname)?;
        let info = readme::find_readme(&repo, &commit)?;
        Ok::<_, ApiError>((refname == commit.id().to_string(), info))
    })
    .await
    .map_err(|err| ApiError::Internal(err.into()))??;
    Ok(sha_addressed_json(immutable, info))
}
