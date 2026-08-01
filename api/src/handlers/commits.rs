//! Handlers for `/repos/{repo}/commits` and the per-commit endpoints.

use std::path::PathBuf;

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use serde::Deserialize;
use utoipa::IntoParams;

use super::{JSON_CONTENT_TYPE, cached_response, parse_limit};
use crate::error::{ApiError, ErrorResponse};
use crate::repo::commits::{CommitDetail, CommitsPage};
use crate::repo::diff::CommitDiff;
use crate::repo::{commits, diff, resolve};
use crate::state::AppState;

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CommitsQuery {
    /// Branch, tag, or commit sha; HEAD when absent. Ignored when `cursor` is set.
    #[serde(rename = "ref")]
    #[param(example = "main")]
    r#ref: Option<String>,
    /// Only commits that changed this file or directory. A path that never
    /// existed yields an empty list rather than a 404.
    path: Option<String>,
    /// `next_cursor` from a previous page, walked from **inclusive**. Opaque:
    /// a malformed or unknown value is `400 invalid_param`, not a 404.
    cursor: Option<String>,
    /// Parsed manually so an invalid value yields the JSON `invalid_param`
    /// envelope instead of axum's plain-text 400. Never clamped.
    #[param(value_type = Option<u32>, minimum = 1, maximum = 100, example = 50)]
    limit: Option<String>,
}

/// Tree lookups need a relative path without empty segments at the ends.
fn clean_path(raw: Option<&str>) -> Option<PathBuf> {
    raw.map(|path| path.trim_matches('/'))
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

/// Commit log
///
/// Cursor-paginated, newest first. Merge commits survive the `path` filter
/// only when the path differs from **every** parent — an approximation of
/// `git log -- <path>` simplification.
#[utoipa::path(
    get,
    path = "/api/v1/repos/{repo}/commits",
    tag = "commits",
    params(
        ("repo" = String, Path, description = "Repository name without the `.git` suffix", example = "git-compose"),
        CommitsQuery,
    ),
    responses(
        (status = 200, description = "One page of commits. An empty repository with no `ref` yields an empty page.", body = CommitsPage,
            headers(
                ("ETag" = String, description = "Validator-derived; opaque"),
                ("Cache-Control" = String, description = "`no-cache`"),
            ),
        ),
        (status = 304, description = "`If-None-Match` matched the current `ETag`"),
        (status = 400, description = "`invalid_param` — bad `limit` or `cursor`", body = ErrorResponse),
        (status = 404, description = "`repo_not_found`, `ref_not_found`", body = ErrorResponse),
    ),
)]
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

/// Commit detail
///
/// Superset of a log entry, plus the full message and the first-parent
/// diffstat. The diffstat has no file cap.
#[utoipa::path(
    get,
    path = "/api/v1/repos/{repo}/commits/{sha}",
    tag = "commits",
    params(
        ("repo" = String, Path, description = "Repository name without the `.git` suffix", example = "git-compose"),
        ("sha" = String, Path, description = "Branch, tag, or commit sha"),
    ),
    responses(
        (status = 200, description = "Commit detail. Immutable caching only when `{sha}` is the resolved full sha (then no `ETag`).", body = CommitDetail,
            headers(
                ("ETag" = String, description = "Validator-derived; absent on full-sha requests"),
                ("Cache-Control" = String, description = "`no-cache`, or `public, max-age=31536000, immutable` for a full sha"),
            ),
        ),
        (status = 304, description = "`If-None-Match` matched the current `ETag`"),
        (status = 404, description = "`repo_not_found`, `ref_not_found`", body = ErrorResponse),
    ),
)]
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

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DiffQuery {
    /// Restrict the diff to one file (literal match, no globbing). A path that
    /// this commit did not touch yields `files: []`, not a 404.
    path: Option<String>,
}

/// Structured commit diff
///
/// Unified diff as JSON (file → hunk → line), against the first parent.
/// Caps: 1000 rendered lines per file (whole hunks are dropped, never cut) and
/// 300 files per response; `additions`/`deletions` stay complete either way.
#[utoipa::path(
    get,
    path = "/api/v1/repos/{repo}/commits/{sha}/diff",
    tag = "commits",
    params(
        ("repo" = String, Path, description = "Repository name without the `.git` suffix", example = "git-compose"),
        ("sha" = String, Path, description = "Branch, tag, or commit sha"),
        DiffQuery,
    ),
    responses(
        (status = 200, description = "Structured diff. Immutable caching only when `{sha}` is the resolved full sha (then no `ETag`).", body = CommitDiff,
            headers(
                ("ETag" = String, description = "Validator-derived; absent on full-sha requests"),
                ("Cache-Control" = String, description = "`no-cache`, or `public, max-age=31536000, immutable` for a full sha"),
            ),
        ),
        (status = 304, description = "`If-None-Match` matched the current `ETag`"),
        (status = 404, description = "`repo_not_found`, `ref_not_found`", body = ErrorResponse),
    ),
)]
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
