//! Handlers for `GET /repos/{repo}/diff`, `/rawdiff`, and `/patch` — arbitrary
//! two-revision diff, plain unified diff, and format-patch output.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, header};
use axum::response::Response;
use serde::Deserialize;
use utoipa::IntoParams;

use super::{
    JSON_CONTENT_TYPE, TEXT_PLAIN_CONTENT_TYPE, cached_response, clean_path, parse_context,
    parse_flag, sanitize_component,
};
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
    /// Skip hunk rendering — `files` stays empty and `truncated` stays
    /// `false`; only `diffstat` (already uncapped) is computed. `context`/
    /// `ignorews` have no effect in this mode.
    #[param(value_type = Option<bool>, example = "1")]
    stat: Option<String>,
}

/// Two-revision diff
///
/// Arbitrary two-revision diff (`git diff <from> <to>`), the same structured
/// shape as the per-commit diff plus an uncapped `diffstat`. `?to=X` alone
/// (no `from`) produces `files` identical to `GET /commits/X/diff`. `from`
/// and `to` accept anything `resolve_commit` does — branch, tag, sha, or a
/// `revparse_single` expression such as `main~1`. `?stat=1` skips hunk
/// rendering entirely (cgit's `dt=2`), returning only the uncapped
/// `diffstat`.
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
        (status = 400, description = "`invalid_param` — bad `context`, `ignorews`, or `stat`", body = ErrorResponse),
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
    let stat_only = parse_flag(query.stat.as_deref(), "stat")?;
    let from_ref = query.from.filter(|value| !value.is_empty());
    // Normalized so `?to=HEAD` and no `to` at all share one cache entry.
    let to_ref = query
        .to
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "HEAD".to_owned());
    let params = format!(
        "from={from_ref:?}&to={to_ref}&path={path:?}&context={context}&ws={ignore_whitespace}&stat={stat_only}"
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
            let rev_diff = if stat_only {
                diff::rev_diff_stat(repo, from_commit.as_ref(), &to_commit, &diff_params)?
            } else {
                diff::rev_diff(repo, from_commit.as_ref(), &to_commit, &diff_params)?
            };
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

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct RawDiffQuery {
    /// Old side of the comparison (`git diff <from> <to>` — not a merge-base
    /// `...` diff). Defaults to `to`'s first parent (the empty tree for a
    /// root commit).
    #[param(example = "main")]
    from: Option<String>,
    /// New side of the comparison. Defaults to `HEAD`.
    #[param(example = "feature/x")]
    to: Option<String>,
    /// Restrict the diff to one file (literal match, no globbing). A path
    /// neither side touched yields an empty diff, not a 404.
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

/// Raw unified diff
///
/// Plain unified diff between two revisions (`git diff <from> <to>`, the same
/// two-dot semantics as `GET /diff` — not a merge-base `...` diff), for
/// `git apply`. **No line/file caps** — unlike the structured diffs, a
/// truncated patch would be a corrupt one. Has no `stat` mode — an empty diff
/// with a stat summary elsewhere wouldn't be a valid patch.
#[utoipa::path(
    get,
    path = "/api/v1/repos/{repo}/rawdiff",
    tag = "diff",
    params(
        ("repo" = String, Path, description = "Repository name without the `.git` suffix", example = "git-compose"),
        RawDiffQuery,
    ),
    responses(
        (status = 200, description = "Plain unified diff, no size limit. Immutable caching only when every side given resolves to the exact requested string as a full sha.",
            content_type = "text/plain", body = String,
            headers(
                ("X-Content-Type-Options" = String, description = "Always `nosniff`"),
                ("ETag" = String, description = "Validator-derived; absent when immutable"),
                ("Cache-Control" = String, description = "`no-cache`, or `public, max-age=31536000, immutable`"),
            ),
        ),
        (status = 304, description = "`If-None-Match` matched the current `ETag`"),
        (status = 400, description = "`invalid_param` — bad `context` or `ignorews`", body = ErrorResponse),
        (status = 404, description = "`repo_not_found`, `ref_not_found` (names the offending side)", body = ErrorResponse),
    ),
)]
pub async fn get_rawdiff(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<RawDiffQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let path = clean_path(query.path.as_deref());
    let context = parse_context(query.context.as_deref())?;
    let ignore_whitespace = parse_flag(query.ignorews.as_deref(), "ignorews")?;
    let from_ref = query.from.filter(|value| !value.is_empty());
    let to_ref = query
        .to
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "HEAD".to_owned());
    let params = format!(
        "from={from_ref:?}&to={to_ref}&path={path:?}&context={context}&ws={ignore_whitespace}"
    );
    let response = cached_response(
        &state,
        &name,
        "rawdiff",
        params,
        TEXT_PLAIN_CONTENT_TYPE,
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
            let body = diff::raw_diff(repo, from_commit.as_ref(), &to_commit, &diff_params)?;
            let to_sha = to_commit.id().to_string();
            let from_sha = from_commit.as_ref().map(|commit| commit.id().to_string());
            let immutable = to_ref == to_sha
                && from_ref
                    .as_deref()
                    .is_none_or(|value| Some(value) == from_sha.as_deref());
            Ok((immutable, body))
        },
    )
    .await?;
    Ok(with_nosniff(response))
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PatchQuery {
    /// Start of the commit range, **excluded** (`git format-patch
    /// <from>..<to>`) — unlike `from` on `/diff`/`/rawdiff`, which names the
    /// other side of a tree comparison. Omitted renders a single patch for
    /// `to` alone.
    #[param(example = "main")]
    from: Option<String>,
    /// End of the range (inclusive). Defaults to `HEAD`.
    #[param(example = "feature/x")]
    to: Option<String>,
    /// Restrict every commit's diff to one file (literal match, no globbing).
    path: Option<String>,
}

/// Format-patch series
///
/// `git format-patch`-style mbox series for the commit range `(from, to]` —
/// `from` **excluded**, the opposite convention from `/diff`/`/rawdiff`.
/// Every commit is diffed against its own first parent, including merges
/// (`git format-patch` itself skips merges; every other axgit diff is
/// first-parent, so this endpoint stays consistent with those instead).
/// Rejects (`400`) rather than truncates a range over
/// [`crate::repo::diff::MAX_PATCH_COMMITS`] commits. For `git am`.
///
/// Includes the author's name and email in `From:` headers — required for
/// `git am` to preserve authorship, and the one axgit response that exposes a
/// plain email address (docs/DECISIONS.md #38). Marked `noindex, nofollow`.
#[utoipa::path(
    get,
    path = "/api/v1/repos/{repo}/patch",
    tag = "diff",
    params(
        ("repo" = String, Path, description = "Repository name without the `.git` suffix", example = "git-compose"),
        PatchQuery,
    ),
    responses(
        (status = 200, description = "mbox-format patch series, no size limit. Immutable caching only when every side given resolves to the exact requested string as a full sha.",
            content_type = "text/plain", body = String,
            headers(
                ("Content-Disposition" = String, description = "`inline; filename=\"{repo}-{safe_to}.patch\"`"),
                ("X-Content-Type-Options" = String, description = "Always `nosniff`"),
                ("X-Robots-Tag" = String, description = "Always `noindex, nofollow`"),
                ("ETag" = String, description = "Validator-derived; absent when immutable"),
                ("Cache-Control" = String, description = "`no-cache`, or `public, max-age=31536000, immutable`"),
            ),
        ),
        (status = 304, description = "`If-None-Match` matched the current `ETag`"),
        (status = 400, description = "`invalid_param` — commit range exceeds the patch-count limit", body = ErrorResponse),
        (status = 404, description = "`repo_not_found`, `ref_not_found` (names the offending side)", body = ErrorResponse),
    ),
)]
pub async fn get_patch(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<PatchQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let path = clean_path(query.path.as_deref());
    let from_ref = query.from.filter(|value| !value.is_empty());
    let to_ref = query
        .to
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "HEAD".to_owned());
    let params = format!("from={from_ref:?}&to={to_ref}&path={path:?}");
    let filename = format!(
        "{}-{}.patch",
        sanitize_component(&name),
        sanitize_component(&to_ref)
    );
    let response = cached_response(
        &state,
        &name,
        "patch",
        params,
        TEXT_PLAIN_CONTENT_TYPE,
        &headers,
        move |repo| {
            let to_commit = resolve::resolve_commit(repo, &to_ref)?;
            let from_commit = from_ref
                .as_deref()
                .map(|refname| resolve::resolve_commit(repo, refname))
                .transpose()?;
            let body = diff::format_patch(repo, from_commit.as_ref(), &to_commit, path.as_deref())?;
            let to_sha = to_commit.id().to_string();
            let from_sha = from_commit.as_ref().map(|commit| commit.id().to_string());
            let immutable = to_ref == to_sha
                && from_ref
                    .as_deref()
                    .is_none_or(|value| Some(value) == from_sha.as_deref());
            Ok((immutable, body))
        },
    )
    .await?;
    Ok(with_patch_headers(response, &filename))
}

fn with_nosniff(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

/// Adds `nosniff`, `Content-Disposition`, and `X-Robots-Tag` to a `/patch`
/// response. Applied post-hoc rather than threading through
/// `cached_response`'s signature — that helper is a hot path shared by every
/// other endpoint. Harmless on a 304, which carries no body.
fn with_patch_headers(mut response: Response, filename: &str) -> Response {
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("inline; filename=\"{filename}\""))
            .expect("sanitized filename is ASCII"),
    );
    headers.insert(
        HeaderName::from_static("x-robots-tag"),
        HeaderValue::from_static("noindex, nofollow"),
    );
    response
}
