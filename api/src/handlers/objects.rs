//! Handlers for `GET /repos/{repo}/objects/{oid}` and `.../objects/{oid}/raw`
//! (api/README.md) — the one by-oid entry point in this API, closing the
//! "Tags and refs" cgit-parity gap (`cgit_object_link()`).

use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Response};

use super::{IMMUTABLE_CACHE_CONTROL, JSON_CONTENT_TYPE, cached_response};
use crate::error::{ApiError, ErrorResponse};
use crate::repo::object::{self, ObjectDetail};
use crate::repo::open;
use crate::state::AppState;

/// Object detail, by id
///
/// Every other route in this API resolves an object through a ref (and, for
/// trees/blobs, a path); this is the one exception. `{oid}` must be a full
/// 40-character hex object id — abbreviations are rejected, since every
/// link axgit itself emits carries a full oid, and requiring one is what
/// lets this endpoint be unconditionally immutable-cached (the address pins
/// the content, unlike `GET /tags/{name}`). Only the `type`-matching payload
/// (`tree`/`blob`/`tag`) is non-null; a commit has none — link to
/// `GET /commits/{sha}` instead.
#[utoipa::path(
    get,
    path = "/api/v1/repos/{repo}/objects/{oid}",
    tag = "files",
    params(
        ("repo" = String, Path, description = "Repository name without the `.git` suffix", example = "git-compose"),
        ("oid" = String, Path, description = "Full 40-character hex object id", example = "94739392266bcbd2a4dcc7e02f57b7cf4ba7ec02"),
    ),
    responses(
        (status = 200, description = "Object detail. Always immutably cached — the address is the content.", body = ObjectDetail,
            headers(
                ("Cache-Control" = String, description = "`public, max-age=31536000, immutable`"),
            ),
        ),
        (status = 400, description = "`invalid_param` — `{oid}` is not a full 40-character hex id", body = ErrorResponse),
        (status = 404, description = "`repo_not_found`, `object_not_found`", body = ErrorResponse),
    ),
)]
pub async fn get_object(
    State(state): State<AppState>,
    Path((name, oid)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let oid = object::parse_full_oid(&oid)?;
    let params = format!("oid={oid}");
    cached_response(
        &state,
        &name,
        "object",
        params,
        JSON_CONTENT_TYPE,
        &headers,
        move |repo| {
            let detail = object::read_object(repo, oid)?;
            // Always immutable — the oid *is* the address.
            Ok((true, serde_json::to_vec(&detail)?))
        },
    )
    .await
}

/// Raw object bytes, by id
///
/// The by-oid analogue of `GET /raw/{ref}/{path}` — blob bytes only, any
/// other kind is `404 object_not_found`. With no filename behind an oid
/// there's no extension to guess a `Content-Type` from, so it's always
/// `text/plain; charset=utf-8` or `application/octet-stream`.
#[utoipa::path(
    get,
    path = "/api/v1/repos/{repo}/objects/{oid}/raw",
    tag = "files",
    params(
        ("repo" = String, Path, description = "Repository name without the `.git` suffix", example = "git-compose"),
        ("oid" = String, Path, description = "Full 40-character hex object id", example = "94739392266bcbd2a4dcc7e02f57b7cf4ba7ec02"),
    ),
    responses(
        (status = 200, description = "Blob bytes. Always immutably cached.",
            content_type = "application/octet-stream",
            body = String,
            headers(
                ("X-Content-Type-Options" = String, description = "Always `nosniff` — repository contents are untrusted"),
                ("Cache-Control" = String, description = "`public, max-age=31536000, immutable`"),
            ),
        ),
        (status = 400, description = "`invalid_param` — `{oid}` is not a full 40-character hex id", body = ErrorResponse),
        (status = 404, description = "`repo_not_found`, `object_not_found` (missing, or not a blob)", body = ErrorResponse),
    ),
)]
pub async fn get_object_raw(
    State(state): State<AppState>,
    Path((name, oid)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let oid = object::parse_full_oid(&oid)?;
    let root = state.config.repo_root.clone();
    let (content_type, bytes) = tokio::task::spawn_blocking(move || {
        let repo = open::open_named(&root, &name)?;
        let raw = object::read_raw(&repo, oid)?;
        let content_type = if raw.binary {
            "application/octet-stream"
        } else {
            "text/plain; charset=utf-8"
        };
        Ok::<_, ApiError>((content_type, raw.bytes))
    })
    .await
    .map_err(|err| ApiError::Internal(err.into()))??;

    let mut response = (
        [
            (header::CONTENT_TYPE, content_type),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        bytes,
    )
        .into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(IMMUTABLE_CACHE_CONTROL),
    );
    Ok(response)
}
