//! Handlers for the file-browsing endpoints: tree, blob, raw, and readme.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use utoipa::IntoParams;

use super::{
    IMMUTABLE_CACHE_CONTROL, JSON_CONTENT_TYPE, NO_CACHE_CONTROL, cached_response, if_none_match,
    not_modified, validator_etag,
};
use crate::error::{ApiError, ErrorResponse};
use crate::repo::blame::BlameInfo;
use crate::repo::blob::BlobInfo;
use crate::repo::readme::ReadmeInfo;
use crate::repo::tree::TreeListing;
use crate::repo::{blame, blob, meta, open, readme, resolve, tree};
use crate::state::AppState;

/// Directory listing
///
/// The `{ref}`/`{path}` boundary is resolved per request by longest-ref
/// matching (`repo::resolve::resolve_ref_path`), because branch and tag names
/// may contain `/`; with no match the first segment is taken as the ref.
/// Omit `{path}` for the root tree.
#[utoipa::path(
    get,
    path = "/api/v1/repos/{repo}/tree/{ref}/{path}",
    tag = "files",
    params(
        ("repo" = String, Path, description = "Repository name without the `.git` suffix", example = "git-compose"),
        ("ref" = String, Path, description = "Branch, tag, or commit sha", example = "main"),
        ("path" = String, Path, description = "Directory path; omit (empty) for the root tree", example = "src"),
    ),
    responses(
        (status = 200, description = "Directory entries, trees first then by name. Immutable caching only when `{ref}` is the resolved full sha (then no `ETag`).", body = TreeListing,
            headers(
                ("ETag" = String, description = "Validator-derived; absent on full-sha requests"),
                ("Cache-Control" = String, description = "`no-cache`, or `public, max-age=31536000, immutable` for a full sha"),
            ),
        ),
        (status = 304, description = "`If-None-Match` matched the current `ETag`"),
        (status = 400, description = "`invalid_param` — `.`, `..`, or an empty path segment", body = ErrorResponse),
        (status = 404, description = "`repo_not_found`, `ref_not_found`, `path_not_found` (missing or not a directory)", body = ErrorResponse),
    ),
)]
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

/// File metadata and content
///
/// `{ref}`/`{path}` split as for the tree endpoint. Binary or over-1 MiB files
/// report `content: null` — fetch the raw endpoint instead.
#[utoipa::path(
    get,
    path = "/api/v1/repos/{repo}/blob/{ref}/{path}",
    tag = "files",
    params(
        ("repo" = String, Path, description = "Repository name without the `.git` suffix", example = "git-compose"),
        ("ref" = String, Path, description = "Branch, tag, or commit sha", example = "main"),
        ("path" = String, Path, description = "File path", example = "README.md"),
    ),
    responses(
        (status = 200, description = "Blob metadata and inline content. Immutable caching only when `{ref}` is the resolved full sha (then no `ETag`).", body = BlobInfo,
            headers(
                ("ETag" = String, description = "Validator-derived; absent on full-sha requests"),
                ("Cache-Control" = String, description = "`no-cache`, or `public, max-age=31536000, immutable` for a full sha"),
            ),
        ),
        (status = 304, description = "`If-None-Match` matched the current `ETag`"),
        (status = 400, description = "`invalid_param` — `.`, `..`, or an empty path segment", body = ErrorResponse),
        (status = 404, description = "`repo_not_found`, `ref_not_found`, `path_not_found` (missing, or a directory/submodule)", body = ErrorResponse),
    ),
)]
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

/// Raw file contents
///
/// No size cap. Not routed through the response cache — raw bodies are
/// unbounded binaries that would crowd out the JSON entries. Non-sha requests
/// still carry an `ETag` so a 304 saves the transfer (the bytes are re-read
/// either way). `{ref}`/`{path}` split as for the tree endpoint.
#[utoipa::path(
    get,
    path = "/api/v1/repos/{repo}/raw/{ref}/{path}",
    tag = "files",
    params(
        ("repo" = String, Path, description = "Repository name without the `.git` suffix", example = "git-compose"),
        ("ref" = String, Path, description = "Branch, tag, or commit sha", example = "main"),
        ("path" = String, Path, description = "File path", example = "README.md"),
    ),
    responses(
        (status = 200, description = "File bytes. `Content-Type` is guessed from the extension, falling back to `text/plain; charset=utf-8` or `application/octet-stream`.",
            content_type = "application/octet-stream",
            body = String,
            headers(
                ("X-Content-Type-Options" = String, description = "Always `nosniff` — repository contents are untrusted"),
                ("ETag" = String, description = "Validator-derived; absent on full-sha requests"),
                ("Cache-Control" = String, description = "`no-cache`, or `public, max-age=31536000, immutable` for a full sha"),
            ),
        ),
        (status = 304, description = "`If-None-Match` matched the current `ETag`"),
        (status = 400, description = "`invalid_param` — `.`, `..`, or an empty path segment", body = ErrorResponse),
        (status = 404, description = "`repo_not_found`, `ref_not_found`, `path_not_found`", body = ErrorResponse),
    ),
)]
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

/// Per-line attribution
///
/// `{ref}`/`{path}` split as for the tree endpoint. Rename tracking is not
/// performed — only moves within the file are attributed, per libgit2
/// defaults.
#[utoipa::path(
    get,
    path = "/api/v1/repos/{repo}/blame/{ref}/{path}",
    tag = "files",
    params(
        ("repo" = String, Path, description = "Repository name without the `.git` suffix", example = "git-compose"),
        ("ref" = String, Path, description = "Branch, tag, or commit sha", example = "main"),
        ("path" = String, Path, description = "File path", example = "src/main.rs"),
    ),
    responses(
        (status = 200, description = "Line ranges covering the whole file. Immutable caching only when `{ref}` is the resolved full sha (then no `ETag`).", body = BlameInfo,
            headers(
                ("ETag" = String, description = "Validator-derived; absent on full-sha requests"),
                ("Cache-Control" = String, description = "`no-cache`, or `public, max-age=31536000, immutable` for a full sha"),
            ),
        ),
        (status = 304, description = "`If-None-Match` matched the current `ETag`"),
        (status = 400, description = "`invalid_param` — `.`, `..`, or an empty path segment", body = ErrorResponse),
        (status = 404, description = "`repo_not_found`, `ref_not_found`, `path_not_found`", body = ErrorResponse),
    ),
)]
pub async fn get_blame(
    State(state): State<AppState>,
    Path((name, rest)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let params = format!("rest={rest}");
    cached_response(
        &state,
        &name,
        "blame",
        params,
        JSON_CONTENT_TYPE,
        &headers,
        move |repo| {
            let split = resolve::resolve_ref_path(repo, &rest)?;
            let info = blame::blame_file(repo, &split.commit, &split.path)?;
            Ok((split.refname == info.sha, serde_json::to_vec(&info)?))
        },
    )
    .await
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ReadmeQuery {
    /// Branch, tag, or commit sha; HEAD when absent.
    #[serde(rename = "ref")]
    #[param(example = "main")]
    r#ref: Option<String>,
}

/// README lookup
///
/// Searches the root tree for `README.md` → `README.rst` → `README.txt` →
/// `README`, matched case-insensitively. Symlinks and binary or over-1 MiB
/// candidates are skipped. Rendering is the frontend's job.
#[utoipa::path(
    get,
    path = "/api/v1/repos/{repo}/readme",
    tag = "files",
    params(
        ("repo" = String, Path, description = "Repository name without the `.git` suffix", example = "git-compose"),
        ReadmeQuery,
    ),
    responses(
        (status = 200, description = "The first usable README. Immutable caching only when `ref` is the resolved full sha (then no `ETag`).", body = ReadmeInfo,
            headers(
                ("ETag" = String, description = "Validator-derived; absent on full-sha requests"),
                ("Cache-Control" = String, description = "`no-cache`, or `public, max-age=31536000, immutable` for a full sha"),
            ),
        ),
        (status = 304, description = "`If-None-Match` matched the current `ETag`"),
        (status = 404, description = "`repo_not_found`, `ref_not_found` (including an empty repository), `path_not_found` (no README)", body = ErrorResponse),
    ),
)]
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
