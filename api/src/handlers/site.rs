//! Handlers for `GET /api/v1/site` (site-wide title, description, and
//! readme, docs/API.md, docs/DECISIONS.md #70) and
//! `GET /api/v1/site/{logo,favicon}` (docs/DECISIONS.md #81).

use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Response};

use super::{
    JSON_CONTENT_TYPE, NO_CACHE_CONTROL, body_etag, etag_response, if_none_match, not_modified,
};
use crate::branding::{self, BrandingAsset};
use crate::error::{ApiError, ErrorResponse};
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

/// Site logo
///
/// 404 when `AXGIT_LOGO` is unset, or when it names an `http(s)://` URL —
/// there is nothing local to serve in that case, since the shell's injected
/// `<meta name="axgit:logo">` already points straight at it
/// (docs/DECISIONS.md #81).
#[utoipa::path(
    get,
    path = "/api/v1/site/logo",
    tag = "site",
    responses(
        (status = 200, description = "Logo image bytes. `Content-Type` is one of `image/svg+xml`, `image/png`, `image/x-icon`, `image/jpeg`, `image/gif`, `image/webp`, `image/avif`, matched from `AXGIT_LOGO`'s extension.",
            content_type = "image/svg+xml",
            headers(
                ("ETag" = String, description = "Hash of the file contents; opaque"),
                ("Cache-Control" = String, description = "`no-cache`"),
                ("X-Content-Type-Options" = String, description = "Always `nosniff`"),
            ),
        ),
        (status = 304, description = "`If-None-Match` matched the current `ETag`"),
        (status = 404, description = "`not_found` — unconfigured, configured as a URL, unreadable, oversized, or an unrecognized extension", body = ErrorResponse),
    ),
)]
pub async fn get_site_logo(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    serve_local_asset(state.config.logo.as_deref(), &headers).await
}

/// Site favicon
///
/// Same rules as [`get_site_logo`], for `AXGIT_FAVICON`.
#[utoipa::path(
    get,
    path = "/api/v1/site/favicon",
    tag = "site",
    responses(
        (status = 200, description = "Favicon image bytes. `Content-Type` is one of `image/svg+xml`, `image/png`, `image/x-icon`, `image/jpeg`, `image/gif`, `image/webp`, `image/avif`, matched from `AXGIT_FAVICON`'s extension.",
            content_type = "image/svg+xml",
            headers(
                ("ETag" = String, description = "Hash of the file contents; opaque"),
                ("Cache-Control" = String, description = "`no-cache`"),
                ("X-Content-Type-Options" = String, description = "Always `nosniff`"),
            ),
        ),
        (status = 304, description = "`If-None-Match` matched the current `ETag`"),
        (status = 404, description = "`not_found` — unconfigured, configured as a URL, unreadable, oversized, or an unrecognized extension", body = ErrorResponse),
    ),
)]
pub async fn get_site_favicon(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    serve_local_asset(state.config.favicon.as_deref(), &headers).await
}

/// Shared by both endpoints above: `raw` is the configured
/// `AXGIT_LOGO`/`AXGIT_FAVICON` value, still in its `http(s)://`-or-path
/// form. Serving only ever applies to the path form — a URL has nothing
/// local to read, and the shell already points straight at it.
async fn serve_local_asset(raw: Option<&str>, headers: &HeaderMap) -> Result<Response, ApiError> {
    let raw = raw.ok_or(ApiError::NotFound)?;
    let BrandingAsset::File(path) = BrandingAsset::parse(raw) else {
        return Err(ApiError::NotFound);
    };
    let asset = branding::read_asset(&path)
        .await
        .ok_or(ApiError::NotFound)?;

    let etag = body_etag(&asset.body);
    if if_none_match(headers, &etag) {
        return Ok(not_modified(&etag));
    }
    Ok((
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static(asset.content_type),
            ),
            (
                header::ETAG,
                HeaderValue::from_str(&etag).expect("generated etags are ascii"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static(NO_CACHE_CONTROL),
            ),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
        ],
        asset.body,
    )
        .into_response())
}
