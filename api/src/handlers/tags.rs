//! Handler for `GET /repos/{repo}/tags/{name}` (api/README.md).

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Response;

use super::{JSON_CONTENT_TYPE, cached_response};
use crate::error::{ApiError, ErrorResponse};
use crate::repo::tag::{self, TagDetail};
use crate::state::AppState;

/// Tag detail
///
/// Full tag message, tagger, and the tag's one-level dereferenced target —
/// closes the gap `GET /refs` leaves open (`tags[].annotation` is only the
/// first line, and `tags[].target` is already the fully peeled commit).
/// Only a real tag name resolves here: a branch name, a commit sha, or
/// `HEAD` all answer `404 ref_not_found` — only `refs/tags/{name}` is
/// consulted.
#[utoipa::path(
    get,
    path = "/api/v1/repos/{repo}/tags/{name}",
    tag = "repos",
    params(
        ("repo" = String, Path, description = "Repository name without the `.git` suffix", example = "git-compose"),
        ("name" = String, Path, description = "Tag name exactly as it appears under `refs/tags` — may itself contain `/`", example = "v1.0.0"),
    ),
    responses(
        (status = 200, description = "Tag detail. Never immutably cached — the URL names a tag ref, not a sha, and a tag can be force-moved onto a different object.", body = TagDetail,
            headers(
                ("ETag" = String, description = "Validator-derived; opaque"),
                ("Cache-Control" = String, description = "`no-cache`"),
            ),
        ),
        (status = 304, description = "`If-None-Match` matched the current `ETag`"),
        (status = 404, description = "`repo_not_found`, `ref_not_found`", body = ErrorResponse),
    ),
)]
pub async fn get_tag(
    State(state): State<AppState>,
    Path((name, tag_name)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let params = format!("tag={tag_name}");
    cached_response(
        &state,
        &name,
        "tag",
        params,
        JSON_CONTENT_TYPE,
        &headers,
        move |repo| Ok((false, serde_json::to_vec(&tag::detail(repo, &tag_name)?)?)),
    )
    .await
}
