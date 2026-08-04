//! Handler for `GET /repos/{repo}/diff` — arbitrary two-revision diff.

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use serde::Deserialize;
use utoipa::IntoParams;

use super::{JSON_CONTENT_TYPE, cached_response, clean_path, parse_context, parse_flag};
use crate::error::{ApiError, ErrorResponse};
use crate::repo::diff::{DiffParams, RevDiff};
use crate::repo::{diff, resolve};
use crate::state::AppState;

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct RevDiffQuery {
    /// Old side of the comparison (`git diff <from> <to>` — not a merge-base
    /// `...` diff). Defaults to `to`'s first parent (the empty tree for a
    /// root commit).
    #[param(example = "main")]
    from: Option<String>,
    /// New side of the comparison. Defaults to `HEAD`.
    #[param(example = "feature/x")]
    to: Option<String>,
    /// Restrict the diff to one file (literal match, no globbing). A path
    /// neither side touched yields `files: []`, not a 404.
    path: Option<String>,
    /// Context lines around each change. Parsed manually so an invalid value
    /// yields the JSON `invalid_param` envelope instead of axum's plain-text
    /// 400. Never clamped.
    #[param(value_type = Option<u32>, minimum = 0, maximum = 100, example = 3)]
    context: Option<String>,
    /// Ignore whitespace-only changes (`git diff --ignore-all-space`).
    #[param(value_type = Option<bool>, example = "1")]
    ignorews: Option<String>,
}

/// Two-revision diff
///
/// Arbitrary two-revision diff (`git diff <from> <to>`), the same structured
/// shape as the per-commit diff plus an uncapped `diffstat`. `?to=X` alone
/// (no `from`) produces `files` identical to `GET /commits/X/diff`. `from`
/// and `to` accept anything `resolve_commit` does — branch, tag, sha, or a
/// `revparse_single` expression such as `main~1`.
#[utoipa::path(
    get,
    path = "/api/v1/repos/{repo}/diff",
    tag = "diff",
    params(
        ("repo" = String, Path, description = "Repository name without the `.git` suffix", example = "git-compose"),
        RevDiffQuery,
    ),
    responses(
        (status = 200, description = "Two-revision diff. Immutable caching only when every side given resolves to the exact requested string as a full sha.", body = RevDiff,
            headers(
                ("ETag" = String, description = "Validator-derived; absent when immutable"),
                ("Cache-Control" = String, description = "`no-cache`, or `public, max-age=31536000, immutable`"),
            ),
        ),
        (status = 304, description = "`If-None-Match` matched the current `ETag`"),
        (status = 400, description = "`invalid_param` — bad `context` or `ignorews`", body = ErrorResponse),
        (status = 404, description = "`repo_not_found`, `ref_not_found` (names the offending side)", body = ErrorResponse),
    ),
)]
pub async fn get_rev_diff(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<RevDiffQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let path = clean_path(query.path.as_deref());
    let context = parse_context(query.context.as_deref())?;
    let ignore_whitespace = parse_flag(query.ignorews.as_deref(), "ignorews")?;
    let from_ref = query.from.filter(|value| !value.is_empty());
    // Normalized so `?to=HEAD` and no `to` at all share one cache entry.
    let to_ref = query
        .to
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "HEAD".to_owned());
    let params = format!(
        "from={from_ref:?}&to={to_ref}&path={path:?}&context={context}&ws={ignore_whitespace}"
    );
    cached_response(
        &state,
        &name,
        "revdiff",
        params,
        JSON_CONTENT_TYPE,
        &headers,
        move |repo| {
            let to_commit = resolve::resolve_commit(repo, &to_ref)?;
            let from_commit = from_ref
                .as_deref()
                .map(|refname| resolve::resolve_commit(repo, refname))
                .transpose()?;
            let diff_params = DiffParams {
                path: path.as_deref(),
                context,
                ignore_whitespace,
            };
            let rev_diff = diff::rev_diff(repo, from_commit.as_ref(), &to_commit, &diff_params)?;
            // Immutable only when every side actually given in the request
            // resolved to itself as a full sha; an omitted `from` inherits
            // whatever `to` resolved to.
            let immutable = to_ref == rev_diff.to
                && from_ref
                    .as_deref()
                    .is_none_or(|value| Some(value) == rev_diff.from.as_deref());
            Ok((immutable, serde_json::to_vec(&rev_diff)?))
        },
    )
    .await
}
