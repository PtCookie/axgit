//! Handlers for the file-browsing endpoints: tree, blob, raw, and readme.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use super::{
    IMMUTABLE_CACHE_CONTROL, JSON_CONTENT_TYPE, NO_CACHE_CONTROL, cached_response, if_none_match,
    not_modified, validator_etag,
};
use crate::error::ApiError;
use crate::repo::{blob, meta, open, readme, resolve, tree};
use crate::state::AppState;

/// `GET /api/v1/repos/{repo}/tree/{ref}/{path...}`
pub async fn get_tree(
    State(state): State<AppState>,
    Path((name, rest)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let params = format!("rest={rest}");
    cached_response(
        &state,
        &name,
        "tree",
        params,
        JSON_CONTENT_TYPE,
        &headers,
        move |repo| {
            let split = resolve::resolve_ref_path(repo, &rest)?;
            let listing = tree::list_tree(repo, &split.commit, &split.path)?;
            Ok((split.refname == listing.sha, serde_json::to_vec(&listing)?))
        },
    )
    .await
}

/// `GET /api/v1/repos/{repo}/blob/{ref}/{path...}`
pub async fn get_blob(
    State(state): State<AppState>,
    Path((name, rest)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let params = format!("rest={rest}");
    cached_response(
        &state,
        &name,
        "blob",
        params,
        JSON_CONTENT_TYPE,
        &headers,
        move |repo| {
            let split = resolve::resolve_ref_path(repo, &rest)?;
            let info = blob::read_blob(repo, &split.commit, &split.path)?;
            Ok((split.refname == info.sha, serde_json::to_vec(&info)?))
        },
    )
    .await
}

/// `GET /api/v1/repos/{repo}/raw/{ref}/{path...}`
///
/// Not routed through the response cache — raw bodies are unbounded binaries
/// that would crowd out the JSON entries. Non-sha requests still carry an
/// `ETag` so a 304 saves the transfer (the bytes are re-read either way).
pub async fn get_raw(
    State(state): State<AppState>,
    Path((name, rest)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let root = state.config.repo_root.clone();
    let (immutable, validator, content_type, bytes) = tokio::task::spawn_blocking(move || {
        let repo = open::open_named(&root, &name)?;
        let validator = meta::validator(&repo);
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
        Ok::<_, ApiError>((immutable, validator, content_type, raw.bytes))
    })
    .await
    .map_err(|err| ApiError::Internal(err.into()))??;

    let etag = (!immutable).then(|| validator_etag(&validator));
    if let Some(etag) = &etag
        && if_none_match(&headers, etag)
    {
        return Ok(not_modified(etag));
    }

    let mut response = (
        [
            (header::CONTENT_TYPE, content_type),
            // Repository contents are untrusted input; never let the browser sniff.
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        bytes,
    )
        .into_response();
    match etag {
        None => {
            response.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static(IMMUTABLE_CACHE_CONTROL),
            );
        }
        Some(etag) => {
            response.headers_mut().insert(
                header::ETAG,
                HeaderValue::from_str(&etag).expect("generated etag is ASCII"),
            );
            response.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static(NO_CACHE_CONTROL),
            );
        }
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
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let params = format!("ref={:?}", query.r#ref);
    cached_response(
        &state,
        &name,
        "readme",
        params,
        JSON_CONTENT_TYPE,
        &headers,
        move |repo| {
            let refname = query.r#ref.as_deref().unwrap_or("HEAD");
            let commit = resolve::resolve_commit(repo, refname)?;
            let info = readme::find_readme(repo, &commit)?;
            Ok((
                refname == commit.id().to_string(),
                serde_json::to_vec(&info)?,
            ))
        },
    )
    .await
}
