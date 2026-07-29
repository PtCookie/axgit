use axum::Json;
use axum::extract::{Path, State};
use serde::Serialize;

use crate::error::ApiError;
use crate::repo::refs::RefsInfo;
use crate::repo::{RepoInfo, RepoSummary, meta, open, refs};
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

/// `GET /api/v1/repos/{repo}`
pub async fn get_repo(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<RepoSummary>, ApiError> {
    let root = state.config.repo_root.clone();
    let clone_url_base = state.config.clone_url_base.clone();
    let summary = tokio::task::spawn_blocking(move || {
        let repo = open::open_named(&root, &name)?;
        let info = meta::read_repo_info(&repo, &name);
        let head = repo
            .head()
            .ok()
            .and_then(|head| head.target())
            .map(|oid| oid.to_string());
        let refs = refs::list_refs(&repo)?;
        let clone_url =
            clone_url_base.map(|base| format!("{}/{name}.git", base.trim_end_matches('/')));
        Ok::<_, ApiError>(RepoSummary {
            name: info.name,
            section: info.section,
            owner: info.owner,
            description: info.description,
            default_branch: info.default_branch,
            last_modified: info.last_modified,
            head,
            branch_count: refs.branches.len(),
            tag_count: refs.tags.len(),
            clone_url,
        })
    })
    .await
    .map_err(|err| ApiError::Internal(err.into()))??;
    Ok(Json(summary))
}

/// `GET /api/v1/repos/{repo}/refs`
pub async fn get_refs(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<RefsInfo>, ApiError> {
    let root = state.config.repo_root.clone();
    let refs = tokio::task::spawn_blocking(move || {
        let repo = open::open_named(&root, &name)?;
        Ok::<_, ApiError>(refs::list_refs(&repo)?)
    })
    .await
    .map_err(|err| ApiError::Internal(err.into()))??;
    Ok(Json(refs))
}
