//! Handler for `GET /api/v1/site` — site-wide title, description, and
//! readme (docs/API.md, docs/DECISIONS.md #70).

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;

use super::{JSON_CONTENT_TYPE, body_etag, etag_response};
use crate::error::ApiError;
use crate::site::{self, SiteInfo};
use crate::state::AppState;

/// Site-wide metadata
///
/// Not tied to a repository — like `GET /api/v1/repos`, this is served with
/// a body-hash `ETag` rather than the HEAD/agefile validator every
/// per-repository endpoint uses.
#[utoipa::path(
    get,
    path = "/api/v1/site",
    tag = "site",
    responses(
        (status = 200, description = "Site metadata", body = SiteInfo,
            headers(
                ("ETag" = String, description = "Hash of the response body; opaque"),
                ("Cache-Control" = String, description = "`no-cache`"),
            ),
        ),
        (status = 304, description = "`If-None-Match` matched the current `ETag`"),
    ),
)]
pub async fn get_site(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let readme = match &state.config.root_readme {
        Some(path) => site::read_root_readme(path).await,
        None => None,
    };
    let info = SiteInfo {
        title: site::effective_title(state.config.root_title.as_deref()),
        description: state.config.root_desc.clone(),
        readme,
    };
    let body = serde_json::to_vec(&info)?;
    let etag = body_etag(&body);
    Ok(etag_response(
        &headers,
        &etag,
        JSON_CONTENT_TYPE,
        body.into(),
    ))
}
