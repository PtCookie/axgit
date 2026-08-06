//! Handler for `/repos/{repo}/stats` — commit-activity graphs (period-
//! bucketed commit counts plus a per-author breakdown), cgit's `stats` page.

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use serde::Deserialize;
use utoipa::IntoParams;

use super::{JSON_CONTENT_TYPE, cached_response, parse_limit};
use crate::error::{ApiError, ErrorResponse};
use crate::repo::resolve;
use crate::repo::stats::{self, StatsPeriod, StatsResults};
use crate::state::AppState;

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct StatsQuery {
    /// `week`, `month` (default), `quarter`, or `year`.
    #[param(value_type = Option<String>, example = "month")]
    period: Option<String>,
    /// Branch, tag, or commit sha; HEAD when absent.
    #[serde(rename = "ref")]
    #[param(example = "main")]
    r#ref: Option<String>,
    /// Number of authors returned, most active first. Parsed manually so an
    /// invalid value yields the JSON `invalid_param` envelope instead of
    /// axum's plain-text 400. Never clamped.
    #[param(value_type = Option<u32>, minimum = 1, maximum = 100, example = 50)]
    limit: Option<String>,
}

fn parse_period(raw: Option<&str>) -> Result<StatsPeriod, ApiError> {
    match raw {
        None | Some("month") => Ok(StatsPeriod::Month),
        Some("week") => Ok(StatsPeriod::Week),
        Some("quarter") => Ok(StatsPeriod::Quarter),
        Some("year") => Ok(StatsPeriod::Year),
        Some(other) => Err(ApiError::InvalidParam(format!(
            "period must be one of week, month, quarter, year (got '{other}')"
        ))),
    }
}

/// Commit-activity statistics
///
/// Buckets commits into 12 `period`-sized windows anchored on the resolved
/// commit's authordate (docs/DECISIONS.md #28), plus a per-author breakdown.
/// git2 in-process scan, same budget approach as search (#26) — not a
/// persistent index or a `git log` exec.
#[utoipa::path(
    get,
    path = "/api/v1/repos/{repo}/stats",
    tag = "repos",
    params(
        ("repo" = String, Path, description = "Repository name without the `.git` suffix", example = "git-compose"),
        StatsQuery,
    ),
    responses(
        (status = 200, description = "Commit statistics. An empty repository with no `ref` yields an empty result.", body = StatsResults,
            headers(
                ("ETag" = String, description = "Validator-derived; absent on full-sha `ref` requests"),
                ("Cache-Control" = String, description = "`no-cache`, or `public, max-age=31536000, immutable` for a full-sha `ref`"),
            ),
        ),
        (status = 304, description = "`If-None-Match` matched the current `ETag`"),
        (status = 400, description = "`invalid_param` — unknown `period`, or bad `limit`", body = ErrorResponse),
        (status = 404, description = "`repo_not_found`, `ref_not_found`", body = ErrorResponse),
    ),
)]
pub async fn get_stats(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<StatsQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let period = parse_period(query.period.as_deref())?;
    let limit = parse_limit(query.limit.as_deref())?;
    let params = format!("period={period:?}&ref={:?}&limit={limit}", query.r#ref);
    cached_response(
        &state,
        &name,
        "stats",
        params,
        JSON_CONTENT_TYPE,
        &headers,
        move |repo| {
            let commit = if let Some(refname) = &query.r#ref {
                Some(resolve::resolve_commit(repo, refname)?)
            } else {
                repo.head().ok().and_then(|head| head.peel_to_commit().ok())
            };
            let Some(commit) = commit else {
                // Empty repository (unborn HEAD): an empty result, not an
                // error — same carve-out the commit log and search apply.
                let results = StatsResults {
                    sha: None,
                    period,
                    truncated: false,
                    author_count: 0,
                    buckets: Vec::new(),
                    authors: Vec::new(),
                    others: None,
                };
                return Ok((false, serde_json::to_vec(&results)?));
            };
            let sha = commit.id().to_string();
            let results = stats::stats(repo, &commit, period, limit)?;
            let immutable = query.r#ref.as_deref() == Some(sha.as_str());
            Ok((immutable, serde_json::to_vec(&results)?))
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_period_should_default_to_month_and_reject_unknown() {
        assert_eq!(parse_period(None).unwrap(), StatsPeriod::Month);
        assert_eq!(parse_period(Some("week")).unwrap(), StatsPeriod::Week);
        assert_eq!(parse_period(Some("month")).unwrap(), StatsPeriod::Month);
        assert_eq!(parse_period(Some("quarter")).unwrap(), StatsPeriod::Quarter);
        assert_eq!(parse_period(Some("year")).unwrap(), StatsPeriod::Year);
        assert!(parse_period(Some("bogus")).is_err());
    }
}
