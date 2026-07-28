pub mod repos;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::error::error_json;

/// Placeholder for endpoints defined in docs/API.md but not implemented yet.
pub async fn not_implemented() -> Response {
    (
        StatusCode::NOT_IMPLEMENTED,
        error_json("not_implemented", "endpoint not implemented yet"),
    )
        .into_response()
}
