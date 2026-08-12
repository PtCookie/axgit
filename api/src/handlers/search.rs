//! Handler for `/repos/{repo}/search` — content, path, commit-message,
//! author/committer, and rev-list-range search within a single repository.

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use serde::Deserialize;
use utoipa::IntoParams;

use super::{DEFAULT_LIMIT, JSON_CONTENT_TYPE, cached_response, parse_limit};
use crate::error::{ApiError, ErrorResponse};
use crate::repo::resolve;
use crate::repo::search::{self, SearchKind, SearchResults};
use crate::state::AppState;

/// `q` is a fixed string (no regex), so a length cap is enough to keep a
/// single query cheap — there is no wildcard blowup risk to guard against.
const MAX_QUERY_CHARS: usize = 200;

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SearchQuery {
    /// Fixed-string, case-insensitive query for every `type` except `range`,
    /// where it is a case-sensitive rev-list expression instead. Trimmed;
    /// empty or over 200 characters is `invalid_param` either way.
    #[param(example = "TODO")]
    q: Option<String>,
    /// `content` (default), `path`, `message`, `author`, `committer`, or
    /// `range`.
    #[serde(rename = "type")]
    #[param(value_type = Option<String>, example = "content")]
    r#type: Option<String>,
    /// Branch, tag, or commit sha; HEAD when absent.
    #[serde(rename = "ref")]
    #[param(example = "main")]
    r#ref: Option<String>,
    /// Parsed manually so an invalid value yields the JSON `invalid_param`
    /// envelope instead of axum's plain-text 400. Never clamped.
    #[param(value_type = Option<u32>, minimum = 1, maximum = 100, example = 50)]
    limit: Option<String>,
}

fn parse_query(raw: Option<&str>) -> Result<String, ApiError> {
    let trimmed = raw.unwrap_or_default().trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_QUERY_CHARS {
        return Err(ApiError::InvalidParam(format!(
            "q must be 1-{MAX_QUERY_CHARS} characters"
        )));
    }
    Ok(trimmed.to_owned())
}

fn parse_kind(raw: Option<&str>) -> Result<SearchKind, ApiError> {
    match raw {
        None | Some("content") => Ok(SearchKind::Content),
        Some("path") => Ok(SearchKind::Path),
        Some("message") => Ok(SearchKind::Message),
        Some("author") => Ok(SearchKind::Author),
        Some("committer") => Ok(SearchKind::Committer),
        Some("range") => Ok(SearchKind::Range),
        Some(other) => Err(ApiError::InvalidParam(format!(
            "type must be one of content, path, message, author, committer, range (got '{other}')"
        ))),
    }
}

/// Repository search
///
/// git2 in-process scan across file content, file paths, commit messages,
/// author/committer names, or a rev-list expression (docs/DECISIONS.md
/// #26/#46) — not a `git grep`/`git log` exec or a persistent index. Every
/// scan is bounded by its own byte/file/commit budget independent of
/// `limit`; either cap sets `truncated: true`.
#[utoipa::path(
    get,
    path = "/api/v1/repos/{repo}/search",
    tag = "search",
    params(
        ("repo" = String, Path, description = "Repository name without the `.git` suffix", example = "git-compose"),
        SearchQuery,
    ),
    responses(
        (status = 200, description = "Search results. An empty repository with no `ref` yields an empty result.", body = SearchResults,
            headers(
                ("ETag" = String, description = "Validator-derived; absent on full-sha `ref` requests"),
                ("Cache-Control" = String, description = "`no-cache`, or `public, max-age=31536000, immutable` for a full-sha `ref` (never immutable for `type=range`)"),
            ),
        ),
        (status = 304, description = "`If-None-Match` matched the current `ETag`"),
        (status = 400, description = "`invalid_param` — missing/oversized `q`, unknown `type`, bad `limit`, or a `range` token starting with `-`", body = ErrorResponse),
        (status = 404, description = "`repo_not_found`, `ref_not_found` (also returned when a `range` token doesn't resolve)", body = ErrorResponse),
    ),
)]
pub async fn get_search(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<SearchQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let q = parse_query(query.q.as_deref())?;
    let kind = parse_kind(query.r#type.as_deref())?;
    let limit = parse_limit(query.limit.as_deref(), DEFAULT_LIMIT)?;
    let params = format!("q={q:?}&type={kind:?}&ref={:?}&limit={limit}", query.r#ref);
    cached_response(
        &state,
        &name,
        "search",
        params,
        JSON_CONTENT_TYPE,
        &headers,
        move |repo| {
            let commit = if let Some(refname) = &query.r#ref {
                Some(resolve::resolve_commit(repo, refname)?)
            } else {
                resolve::default_commit(repo)
            };
            let Some(commit) = commit else {
                // Empty repository (unborn HEAD): an empty result, not an
                // error — same carve-out the commit log applies.
                let results = SearchResults {
                    sha: None,
                    kind,
                    truncated: false,
                    files: Vec::new(),
                    commits: Vec::new(),
                };
                return Ok((false, serde_json::to_vec(&results)?));
            };
            let sha = commit.id().to_string();
            let results = search::search(repo, &commit, kind, &q, limit)?;
            // `range` walks revisions named in `q` (e.g. `main~5..main`),
            // which can move independently of the resolved `ref`/`sha` — so
            // it never qualifies for immutable caching, unlike every other
            // variant (docs/DECISIONS.md #46).
            let immutable =
                kind != SearchKind::Range && query.r#ref.as_deref() == Some(sha.as_str());
            Ok((immutable, serde_json::to_vec(&results)?))
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_query_should_reject_empty_and_oversized() {
        assert!(parse_query(None).is_err());
        assert!(parse_query(Some("   ")).is_err());
        assert!(parse_query(Some(&"a".repeat(201))).is_err());
        assert_eq!(parse_query(Some("  hi  ")).unwrap(), "hi");
    }

    #[test]
    fn parse_kind_should_default_to_content_and_reject_unknown() {
        assert_eq!(parse_kind(None).unwrap(), SearchKind::Content);
        assert_eq!(parse_kind(Some("content")).unwrap(), SearchKind::Content);
        assert_eq!(parse_kind(Some("path")).unwrap(), SearchKind::Path);
        assert_eq!(parse_kind(Some("message")).unwrap(), SearchKind::Message);
        assert_eq!(parse_kind(Some("author")).unwrap(), SearchKind::Author);
        assert_eq!(
            parse_kind(Some("committer")).unwrap(),
            SearchKind::Committer
        );
        assert_eq!(parse_kind(Some("range")).unwrap(), SearchKind::Range);
        assert!(parse_kind(Some("bogus")).is_err());
    }
}
