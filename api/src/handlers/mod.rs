pub mod archive;
pub mod commits;
pub mod files;
pub mod repos;

use axum::Json;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::error::error_json;

/// `Cache-Control` for responses addressed by a full commit sha (docs/API.md).
pub(crate) const IMMUTABLE_CACHE_CONTROL: &str = "public, max-age=31536000, immutable";

/// Wraps `body` as JSON, marked immutable only when the request addressed the
/// commit by its full sha — ref parameters also resolve branches, tags, and
/// abbreviated shas, and those responses are not immutable.
pub(crate) fn sha_addressed_json<T: Serialize>(immutable: bool, body: T) -> Response {
    let json = Json(body);
    if immutable {
        ([(header::CACHE_CONTROL, IMMUTABLE_CACHE_CONTROL)], json).into_response()
    } else {
        json.into_response()
    }
}

/// Placeholder for endpoints defined in docs/API.md but not implemented yet.
pub async fn not_implemented() -> Response {
    (
        StatusCode::NOT_IMPLEMENTED,
        error_json("not_implemented", "endpoint not implemented yet"),
    )
        .into_response()
}
