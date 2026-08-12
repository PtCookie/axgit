use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::{JSON_CONTENT_TYPE, body_etag, cached_response, etag_response};
use crate::error::{ApiError, ErrorResponse};
use crate::repo::refs::RefsInfo;
use crate::repo::sort::{RepoOrder, sort_repos};
use crate::repo::{RepoInfo, RepoSummary, meta, refs};
use crate::state::AppState;

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ReposQuery {
    /// `name` (default), `desc`, `owner`, `idle`, or `section`, optionally
    /// prefixed with `-` to reverse direction (`idle` reverses to ascending
    /// under `-idle`, i.e. oldest first). Falls back to
    /// `AXGIT_REPOSITORY_SORT` (server default `name`) when absent.
    #[param(value_type = Option<String>, example = "idle")]
    sort: Option<String>,
}

/// Parses `?sort=`, falling back to `default` (the server-configured order)
/// when the param is absent. Parsed manually — see [`super::parse_limit`].
pub(crate) fn parse_sort(raw: Option<&str>, default: RepoOrder) -> Result<RepoOrder, ApiError> {
    let Some(raw) = raw else {
        return Ok(default);
    };
    RepoOrder::parse(raw).map_err(|_| {
        ApiError::InvalidParam(format!(
            "sort must be one of name, desc, owner, idle, section, optionally prefixed with '-' \
             (got '{raw}')"
        ))
    })
}

#[derive(Serialize, ToSchema)]
pub struct ReposResponse {
    pub repos: Vec<RepoInfo>,
    /// The effective sort order actually applied — the request's `?sort=` if
    /// given, else the server's configured default. Lets a client mark the
    /// active column without knowing `AXGIT_REPOSITORY_SORT`.
    #[schema(example = "name")]
    pub sort: String,
}

/// List repositories
///
/// Served from the scan snapshot ([`crate::cache::ScanCache`]), not the
/// response cache — the list has no single backing repository, so its `ETag`
/// is a hash of the serialized body. `sort` changes the body and therefore
/// the `ETag`, so no extra cache-key work is needed for it.
#[utoipa::path(
    get,
    path = "/api/v1/repos",
    tag = "repos",
    params(ReposQuery),
    responses(
        (status = 200, description = "Repository list", body = ReposResponse,
            headers(
                ("ETag" = String, description = "Hash of the response body; opaque"),
                ("Cache-Control" = String, description = "`no-cache`"),
            ),
        ),
        (status = 304, description = "`If-None-Match` matched the current `ETag`"),
        (status = 400, description = "`invalid_param` — unknown `sort`", body = ErrorResponse),
        (status = 500, description = "Repository root could not be scanned", body = ErrorResponse),
    ),
)]
pub async fn list_repos(
    State(state): State<AppState>,
    Query(query): Query<ReposQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let order = parse_sort(query.sort.as_deref(), state.config.repository_sort)?;
    let repos = state
        .scan_cache
        .get_or_scan(&state.config.repo_root)
        .await?;
    let mut repos = repos.as_ref().clone();
    sort_repos(&mut repos, order);
    let body = serde_json::to_vec(&ReposResponse {
        repos,
        sort: order.to_string(),
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
                homepage: info.homepage,
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
