use std::path::PathBuf;

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::repo::commits::CommitsPage;
use crate::repo::refs::RefsInfo;
use crate::repo::{RepoInfo, RepoSummary, commits, meta, open, refs, resolve};
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

#[derive(Deserialize)]
pub struct CommitsQuery {
    /// Branch, tag, or commit sha; HEAD when absent. Ignored when `cursor` is set.
    #[serde(rename = "ref")]
    r#ref: Option<String>,
    path: Option<String>,
    cursor: Option<String>,
    /// Parsed manually so an invalid value yields the JSON `invalid_param`
    /// envelope instead of axum's plain-text 400.
    limit: Option<String>,
}

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 100;

fn parse_limit(raw: Option<&str>) -> Result<usize, ApiError> {
    let Some(raw) = raw else {
        return Ok(DEFAULT_LIMIT);
    };
    match raw.parse::<usize>() {
        Ok(limit) if (1..=MAX_LIMIT).contains(&limit) => Ok(limit),
        _ => Err(ApiError::InvalidParam(format!(
            "limit must be an integer between 1 and {MAX_LIMIT}"
        ))),
    }
}

/// `GET /api/v1/repos/{repo}/commits?ref=&path=&cursor=&limit=`
pub async fn list_commits(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<CommitsQuery>,
) -> Result<Json<CommitsPage>, ApiError> {
    let limit = parse_limit(query.limit.as_deref())?;
    // Tree lookups need a relative path without empty segments at the ends.
    let path = query
        .path
        .as_deref()
        .map(|path| path.trim_matches('/'))
        .filter(|path| !path.is_empty())
        .map(PathBuf::from);
    let root = state.config.repo_root.clone();
    let page = tokio::task::spawn_blocking(move || {
        let repo = open::open_named(&root, &name)?;
        let start = if let Some(cursor) = &query.cursor {
            // The cursor is an opaque token from a previous response, so any
            // failure is a malformed request (400), not a missing ref (404).
            git2::Oid::from_str(cursor)
                .ok()
                .and_then(|oid| repo.find_commit(oid).ok())
                .ok_or_else(|| ApiError::InvalidParam(format!("invalid cursor '{cursor}'")))?
                .id()
        } else if let Some(refname) = &query.r#ref {
            resolve::resolve_commit(&repo, refname)?.id()
        } else {
            match repo.head().ok().and_then(|head| head.peel_to_commit().ok()) {
                Some(commit) => commit.id(),
                // Empty repository (unborn HEAD): an empty page, not an error.
                None => {
                    return Ok(CommitsPage {
                        commits: Vec::new(),
                        next_cursor: None,
                    });
                }
            }
        };
        commits::log(&repo, start, path.as_deref(), limit)
    })
    .await
    .map_err(|err| ApiError::Internal(err.into()))??;
    Ok(Json(page))
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
