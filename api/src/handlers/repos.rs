use axum::Json;
use axum::extract::State;
use serde::Serialize;

use crate::error::ApiError;
use crate::repo::RepoInfo;
use crate::state::AppState;

#[derive(Serialize)]
pub struct ReposResponse {
    pub repos: Vec<RepoInfo>,
}

/// `GET /api/v1/repos`
pub async fn list_repos(State(state): State<AppState>) -> Result<Json<ReposResponse>, ApiError> {
    let repos = state
        .scan_cache
        .get_or_scan(&state.config.repo_root)
        .await?;
    Ok(Json(ReposResponse {
        repos: repos.as_ref().clone(),
    }))
}
