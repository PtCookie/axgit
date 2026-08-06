use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Response;
use serde::Serialize;
use utoipa::ToSchema;

use super::{JSON_CONTENT_TYPE, body_etag, cached_response, etag_response};
use crate::error::{ApiError, ErrorResponse};
use crate::repo::refs::RefsInfo;
use crate::repo::{RepoInfo, RepoSummary, meta, refs};
use crate::state::AppState;

#[derive(Serialize, ToSchema)]
pub struct ReposResponse {
    pub repos: Vec<RepoInfo>,
}

/// List repositories
///
/// Served from the scan snapshot ([`crate::cache::ScanCache`]), not the
/// response cache — the list has no single backing repository, so its `ETag`
/// is a hash of the serialized body.
#[utoipa::path(
    get,
    path = "/api/v1/repos",
    tag = "repos",
    responses(
        (status = 200, description = "Repository list", body = ReposResponse,
            headers(
                ("ETag" = String, description = "Hash of the response body; opaque"),
                ("Cache-Control" = String, description = "`no-cache`"),
            ),
        ),
        (status = 304, description = "`If-None-Match` matched the current `ETag`"),
        (status = 500, description = "Repository root could not be scanned", body = ErrorResponse),
    ),
)]
pub async fn list_repos(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let repos = state
        .scan_cache
        .get_or_scan(&state.config.repo_root)
        .await?;
    let body = serde_json::to_vec(&ReposResponse {
        repos: repos.as_ref().clone(),
    })?;
    let etag = body_etag(&body);
    Ok(etag_response(
        &headers,
        &etag,
        JSON_CONTENT_TYPE,
        body.into(),
    ))
}

/// Repository summary
///
/// An empty repository (unborn HEAD) still answers `200`, with `head`,
/// `default_branch` and `last_modified` `null` and both counts 0.
#[utoipa::path(
    get,
    path = "/api/v1/repos/{repo}",
    tag = "repos",
    params(
        ("repo" = String, Path, description = "Repository name without the `.git` suffix", example = "git-compose"),
    ),
    responses(
        (status = 200, description = "Repository summary", body = RepoSummary,
            headers(
                ("ETag" = String, description = "Validator-derived; opaque"),
                ("Cache-Control" = String, description = "`no-cache`"),
            ),
        ),
        (status = 304, description = "`If-None-Match` matched the current `ETag`"),
        (status = 404, description = "`repo_not_found`", body = ErrorResponse),
    ),
)]
pub async fn get_repo(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let clone_url_base = state.config.clone_url_base.clone();
    let compute_name = name.clone();
    cached_response(
        &state,
        &name,
        "summary",
        String::new(),
        JSON_CONTENT_TYPE,
        &headers,
        move |repo| {
            let info = meta::read_repo_info(repo, &compute_name);
            let head = repo
                .head()
                .ok()
                .and_then(|head| head.target())
                .map(|oid| oid.to_string());
            let refs = refs::list_refs(repo)?;
            let clone_url = clone_url_base
                .map(|base| format!("{}/{compute_name}.git", base.trim_end_matches('/')));
            let summary = RepoSummary {
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
            };
            Ok((false, serde_json::to_vec(&summary)?))
        },
    )
    .await
}

/// Branches and tags
///
/// Tag targets are peeled to the commit, so an annotated tag reports the
/// commit sha rather than the tag object. `remote_branches` is `[]` on
/// essentially every repository — see its field doc.
#[utoipa::path(
    get,
    path = "/api/v1/repos/{repo}/refs",
    tag = "repos",
    params(
        ("repo" = String, Path, description = "Repository name without the `.git` suffix", example = "git-compose"),
    ),
    responses(
        (status = 200, description = "Local branches, remote-tracking branches, and tags, each sorted by name", body = RefsInfo,
            headers(
                ("ETag" = String, description = "Validator-derived; opaque"),
                ("Cache-Control" = String, description = "`no-cache`"),
            ),
        ),
        (status = 304, description = "`If-None-Match` matched the current `ETag`"),
        (status = 404, description = "`repo_not_found`", body = ErrorResponse),
    ),
)]
pub async fn get_refs(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    cached_response(
        &state,
        &name,
        "refs",
        String::new(),
        JSON_CONTENT_TYPE,
        &headers,
        |repo| {
            let refs = refs::list_refs(repo)?;
            Ok((false, serde_json::to_vec(&refs)?))
        },
    )
    .await
}
