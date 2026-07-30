use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// API error mapped to the JSON error contract in docs/API.md.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("repository '{0}' not found")]
    RepoNotFound(String),
    #[error("ref '{0}' not found")]
    RefNotFound(String),
    #[error("path '{0}' not found")]
    PathNotFound(String),
    #[error("invalid parameter: {0}")]
    InvalidParam(String),
    #[error("repository is read-only over HTTP; push via SSH")]
    ReadOnly,
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl From<git2::Error> for ApiError {
    fn from(err: git2::Error) -> Self {
        Self::Internal(err.into())
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(err: serde_json::Error) -> Self {
        Self::Internal(err.into())
    }
}

impl ApiError {
    fn status_and_code(&self) -> (StatusCode, &'static str) {
        match self {
            Self::RepoNotFound(_) => (StatusCode::NOT_FOUND, "repo_not_found"),
            Self::RefNotFound(_) => (StatusCode::NOT_FOUND, "ref_not_found"),
            Self::PathNotFound(_) => (StatusCode::NOT_FOUND, "path_not_found"),
            Self::InvalidParam(_) => (StatusCode::BAD_REQUEST, "invalid_param"),
            Self::ReadOnly => (StatusCode::FORBIDDEN, "read_only"),
            Self::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = self.status_and_code();
        // Internal errors go to the log in full; the client only sees a generic message.
        let message = if let Self::Internal(err) = &self {
            tracing::error!(error = format!("{err:#}"), "internal error");
            "internal server error".to_owned()
        } else {
            self.to_string()
        };
        (status, error_json(code, &message)).into_response()
    }
}

/// Builds the `{"error": {"code", "message"}}` body shared by all error responses.
pub fn error_json(code: &str, message: &str) -> Json<serde_json::Value> {
    Json(json!({ "error": { "code": code, "message": message } }))
}
