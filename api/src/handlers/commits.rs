//! Handlers for `/repos/{repo}/commits` and the per-commit endpoints.

use std::path::PathBuf;

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use serde::Deserialize;

use super::{JSON_CONTENT_TYPE, cached_response};
use crate::error::ApiError;
use crate::repo::commits::CommitsPage;
use crate::repo::{commits, diff, resolve};
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
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let limit = parse_limit(query.limit.as_deref())?;
    let path = clean_path(query.path.as_deref());
    // A missing `ref` is not normalized to HEAD: the two take different
    // unborn-HEAD paths (empty page vs 404), so they stay distinct keys.
    let params = format!(
        "ref={:?}&path={:?}&cursor={:?}&limit={limit}",
        query.r#ref, path, query.cursor
    );
    cached_response(
        &state,
        &name,
        "commits",
        params,
        JSON_CONTENT_TYPE,
        &headers,
        move |repo| {
            let start = if let Some(cursor) = &query.cursor {
                // The cursor is an opaque token from a previous response, so any
                // failure is a malformed request (400), not a missing ref (404).
                git2::Oid::from_str(cursor)
                    .ok()
                    .and_then(|oid| repo.find_commit(oid).ok())
                    .ok_or_else(|| ApiError::InvalidParam(format!("invalid cursor '{cursor}'")))?
                    .id()
            } else if let Some(refname) = &query.r#ref {
                resolve::resolve_commit(repo, refname)?.id()
            } else {
                match repo.head().ok().and_then(|head| head.peel_to_commit().ok()) {
                    Some(commit) => commit.id(),
                    // Empty repository (unborn HEAD): an empty page, not an error.
                    None => {
                        let page = CommitsPage {
                            commits: Vec::new(),
                            next_cursor: None,
                        };
                        return Ok((false, serde_json::to_vec(&page)?));
                    }
                }
            };
            let page = commits::log(repo, start, path.as_deref(), limit)?;
            Ok((false, serde_json::to_vec(&page)?))
        },
    )
    .await
}

/// `GET /api/v1/repos/{repo}/commits/{sha}`
pub async fn get_commit(
    State(state): State<AppState>,
    Path((name, sha)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let params = format!("sha={sha}");
    cached_response(
        &state,
        &name,
        "commit",
        params,
        JSON_CONTENT_TYPE,
        &headers,
        move |repo| {
            let commit = resolve::resolve_commit(repo, &sha)?;
            let detail = commits::detail(repo, &commit)?;
            Ok((sha == detail.sha, serde_json::to_vec(&detail)?))
        },
    )
    .await
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
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let path = clean_path(query.path.as_deref());
    let params = format!("sha={sha}&path={path:?}");
    cached_response(
        &state,
        &name,
        "diff",
        params,
        JSON_CONTENT_TYPE,
        &headers,
        move |repo| {
            let commit = resolve::resolve_commit(repo, &sha)?;
            let commit_diff = diff::commit_diff(repo, &commit, path.as_deref())?;
            Ok((sha == commit_diff.sha, serde_json::to_vec(&commit_diff)?))
        },
    )
    .await
}
