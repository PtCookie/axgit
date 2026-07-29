//! Handlers for `/repos/{repo}/commits` and the per-commit endpoints.

use std::path::PathBuf;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::Response;
use serde::Deserialize;

use super::sha_addressed_json;
use crate::error::ApiError;
use crate::repo::commits::CommitsPage;
use crate::repo::{commits, diff, open, resolve};
use crate::state::AppState;

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

/// Tree lookups need a relative path without empty segments at the ends.
fn clean_path(raw: Option<&str>) -> Option<PathBuf> {
    raw.map(|path| path.trim_matches('/'))
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

/// `GET /api/v1/repos/{repo}/commits?ref=&path=&cursor=&limit=`
pub async fn list_commits(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<CommitsQuery>,
) -> Result<Json<CommitsPage>, ApiError> {
    let limit = parse_limit(query.limit.as_deref())?;
    let path = clean_path(query.path.as_deref());
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

/// `GET /api/v1/repos/{repo}/commits/{sha}`
pub async fn get_commit(
    State(state): State<AppState>,
    Path((name, sha)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let root = state.config.repo_root.clone();
    let requested = sha.clone();
    let detail = tokio::task::spawn_blocking(move || {
        let repo = open::open_named(&root, &name)?;
        let commit = resolve::resolve_commit(&repo, &sha)?;
        commits::detail(&repo, &commit)
    })
    .await
    .map_err(|err| ApiError::Internal(err.into()))??;
    Ok(sha_addressed_json(requested == detail.sha, detail))
}

#[derive(Deserialize)]
pub struct DiffQuery {
    path: Option<String>,
}

/// `GET /api/v1/repos/{repo}/commits/{sha}/diff?path=`
pub async fn get_commit_diff(
    State(state): State<AppState>,
    Path((name, sha)): Path<(String, String)>,
    Query(query): Query<DiffQuery>,
) -> Result<Response, ApiError> {
    let path = clean_path(query.path.as_deref());
    let root = state.config.repo_root.clone();
    let requested = sha.clone();
    let commit_diff = tokio::task::spawn_blocking(move || {
        let repo = open::open_named(&root, &name)?;
        let commit = resolve::resolve_commit(&repo, &sha)?;
        diff::commit_diff(&repo, &commit, path.as_deref())
    })
    .await
    .map_err(|err| ApiError::Internal(err.into()))??;
    Ok(sha_addressed_json(
        requested == commit_diff.sha,
        commit_diff,
    ))
}
