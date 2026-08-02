//! Integration tests for `GET /api/v1/repos/{repo}/stats`.

mod common;

use std::path::Path;

use axum::http::StatusCode;
use serde_json::Value;
use tempfile::TempDir;

use common::router_for;

/// One commit of [`setup`], with a controlled author (unlike
/// `common::CommitSpec`, which always commits as "Test Author").
struct Commit {
    file: &'static str,
    content: &'static str,
    message: &'static str,
    /// RFC 3339, used for both author and committer date.
    date: &'static str,
    author_name: &'static str,
    author_email: &'static str,
}

/// `beta.git`; shas oldest → newest, all on a linear history:
/// 0. 2026-03-10, "Test Author"
/// 1. 2026-05-20, "Test Author"
/// 2. 2026-07-01, "Alice"
/// 3. 2026-07-15 (tip — the window anchor), "Bob"
///
/// So `period=month` buckets two March/May single commits, one July pair
/// (Alice + Bob), across three distinct authors.
fn setup() -> (TempDir, Vec<String>) {
    let root = tempfile::tempdir().expect("failed to create fixture root");
    let bare = common::create_bare_repo(root.path(), "beta.git");
    let work = tempfile::tempdir().expect("failed to create work dir");
    let work_path = work.path();
    common::git(
        work_path,
        &["clone", "--quiet", bare.to_str().unwrap(), "."],
    );

    let commits = [
        Commit {
            file: "a.txt",
            content: "one",
            message: "root",
            date: "2026-03-10T09:00:00Z",
            author_name: "Test Author",
            author_email: "author@example.com",
        },
        Commit {
            file: "b.txt",
            content: "two",
            message: "second",
            date: "2026-05-20T09:00:00Z",
            author_name: "Test Author",
            author_email: "author@example.com",
        },
        Commit {
            file: "c.txt",
            content: "three",
            message: "third",
            date: "2026-07-01T09:00:00Z",
            author_name: "Alice",
            author_email: "alice@example.com",
        },
        Commit {
            file: "d.txt",
            content: "four",
            message: "fourth",
            date: "2026-07-15T10:00:00Z",
            author_name: "Bob",
            author_email: "bob@example.com",
        },
    ];

    let mut shas = Vec::new();
    for spec in &commits {
        std::fs::write(work_path.join(spec.file), spec.content).unwrap();
        common::git(work_path, &["add", "."]);
        common::git_output(
            work_path,
            &["commit", "--quiet", "-m", spec.message],
            &[
                ("GIT_AUTHOR_NAME", spec.author_name),
                ("GIT_AUTHOR_EMAIL", spec.author_email),
                ("GIT_AUTHOR_DATE", spec.date),
                ("GIT_COMMITTER_DATE", spec.date),
            ],
        );
        shas.push(common::git_output(work_path, &["rev-parse", "HEAD"], &[]));
    }
    common::git(work_path, &["push", "--quiet", "origin", "HEAD:main"]);
    (root, shas)
}

async fn get_ok(root: &Path, uri: &str) -> Value {
    let (status, json) = common::get_json(router_for(root), uri).await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");
    json
}

/// Commits for the bucket starting at `start` (an RFC 3339 UTC instant, e.g.
/// `"2026-07-01T00:00:00+00:00"`), or `None` if no bucket has that start.
fn bucket_commits(json: &Value, start: &str) -> Option<i64> {
    json["buckets"]
        .as_array()
        .expect("buckets missing")
        .iter()
        .find(|bucket| bucket["start"] == start)
        .map(|bucket| bucket["commits"].as_i64().unwrap())
}

#[tokio::test]
async fn stats_buckets_commits_by_month_anchored_on_the_tip_authordate() {
    let (root, shas) = setup();
    let json = get_ok(root.path(), "/api/v1/repos/beta/stats").await;

    assert_eq!(json["sha"].as_str(), Some(shas[3].as_str()));
    assert_eq!(json["period"].as_str(), Some("month"));
    assert_eq!(json["truncated"].as_bool(), Some(false));
    assert_eq!(json["author_count"].as_i64(), Some(3));
    assert_eq!(json["buckets"].as_array().map(Vec::len), Some(12));

    assert_eq!(bucket_commits(&json, "2026-03-01T00:00:00+00:00"), Some(1));
    assert_eq!(bucket_commits(&json, "2026-05-01T00:00:00+00:00"), Some(1));
    assert_eq!(bucket_commits(&json, "2026-07-01T00:00:00+00:00"), Some(2));
    // The window's oldest bucket, one year back from the tip's month.
    assert_eq!(bucket_commits(&json, "2025-08-01T00:00:00+00:00"), Some(0));
    let total: i64 = json["buckets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|bucket| bucket["commits"].as_i64().unwrap())
        .sum();
    assert_eq!(total, 4, "unexpected response: {json}");

    let authors = json["authors"].as_array().expect("authors missing");
    assert_eq!(authors.len(), 3);
    // "Test Author" has 2 commits (March + May) — the most active, so first.
    assert_eq!(authors[0]["author"]["name"], "Test Author");
    assert_eq!(authors[0]["commits"], 2);
    assert_eq!(
        authors[0]["buckets"].as_array().map(Vec::len),
        Some(12),
        "author buckets must be parallel to the top-level buckets array"
    );
    // Alice and Bob tie at 1 commit each — order between them isn't asserted,
    // just that both are present.
    let remaining: std::collections::HashSet<&str> = authors[1..]
        .iter()
        .map(|author| author["author"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(remaining, std::collections::HashSet::from(["Alice", "Bob"]));
    assert!(authors[1..].iter().all(|author| author["commits"] == 1));
}

#[tokio::test]
async fn stats_defaults_to_month_period() {
    let (root, _shas) = setup();
    let default = get_ok(root.path(), "/api/v1/repos/beta/stats").await;
    let explicit = get_ok(root.path(), "/api/v1/repos/beta/stats?period=month").await;
    assert_eq!(default, explicit);
}

#[tokio::test]
async fn stats_accepts_every_period_value() {
    let (root, _shas) = setup();
    for period in ["week", "month", "quarter", "year"] {
        let json = get_ok(
            root.path(),
            &format!("/api/v1/repos/beta/stats?period={period}"),
        )
        .await;
        assert_eq!(json["period"].as_str(), Some(period), "period {period}");
        assert_eq!(json["buckets"].as_array().map(Vec::len), Some(12));
    }
}

#[tokio::test]
async fn stats_rejects_unknown_period() {
    let (root, _shas) = setup();
    let (status, json) = common::get_json(
        router_for(root.path()),
        "/api/v1/repos/beta/stats?period=bogus",
    )
    .await;
    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("invalid_param")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn stats_rejects_invalid_limit() {
    let (root, _shas) = setup();
    for limit in ["0", "101", "abc"] {
        let uri = format!("/api/v1/repos/beta/stats?limit={limit}");
        let (status, json) = common::get_json(router_for(root.path()), &uri).await;
        assert_eq!(
            (status, json["error"]["code"].as_str()),
            (StatusCode::BAD_REQUEST, Some("invalid_param")),
            "limit={limit} was not rejected: {json}"
        );
    }
}

#[tokio::test]
async fn stats_returns_404_for_unknown_ref() {
    let (root, _shas) = setup();
    let (status, json) = common::get_json(
        router_for(root.path()),
        "/api/v1/repos/beta/stats?ref=no-such-ref",
    )
    .await;
    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("ref_not_found")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn stats_returns_404_for_unknown_repo() {
    let root = tempfile::tempdir().unwrap();
    let (status, json) =
        common::get_json(router_for(root.path()), "/api/v1/repos/nope/stats").await;
    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("repo_not_found")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn stats_returns_empty_result_for_empty_repo() {
    let root = tempfile::tempdir().expect("failed to create fixture root");
    common::create_bare_repo(root.path(), "empty.git");

    let json = get_ok(root.path(), "/api/v1/repos/empty/stats").await;
    assert_eq!(
        (
            json["sha"].as_null(),
            json["truncated"].as_bool(),
            json["author_count"].as_i64(),
            json["buckets"].as_array().map(Vec::len),
            json["authors"].as_array().map(Vec::len),
        ),
        (Some(()), Some(false), Some(0), Some(0), Some(0)),
        "unexpected response: {json}"
    );

    // An explicit ref on an empty repo is a resolution failure, not an empty result.
    let (status, json) = common::get_json(
        router_for(root.path()),
        "/api/v1/repos/empty/stats?ref=main",
    )
    .await;
    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("ref_not_found")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn stats_applies_limit_and_reports_truncation_without_shrinking_bucket_totals() {
    let (root, _shas) = setup();
    let json = get_ok(root.path(), "/api/v1/repos/beta/stats?limit=1").await;
    assert_eq!(json["author_count"].as_i64(), Some(3));
    assert_eq!(json["authors"].as_array().map(Vec::len), Some(1));
    assert_eq!(json["truncated"].as_bool(), Some(true));
    let total: i64 = json["buckets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|bucket| bucket["commits"].as_i64().unwrap())
        .sum();
    assert_eq!(
        total, 4,
        "bucket totals must include commits from authors cut by limit"
    );
}

#[tokio::test]
async fn stats_sets_immutable_cache_for_full_sha_ref_only() {
    let (root, shas) = setup();
    let (status, headers, json) = common::get_json_with_headers(
        router_for(root.path()),
        &format!("/api/v1/repos/beta/stats?ref={}", shas[3]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");
    assert_eq!(
        headers
            .get("cache-control")
            .map(|value| value.to_str().unwrap()),
        Some("public, max-age=31536000, immutable")
    );
    assert!(!headers.contains_key("etag"));

    let (status, headers, json) =
        common::get_json_with_headers(router_for(root.path()), "/api/v1/repos/beta/stats?ref=main")
            .await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");
    assert_eq!(
        headers
            .get("cache-control")
            .map(|value| value.to_str().unwrap()),
        Some("no-cache")
    );
    assert!(headers.contains_key("etag"));
}

#[tokio::test]
async fn stats_honors_if_none_match() {
    let (root, _shas) = setup();
    let (_, headers, _) =
        common::get_json_with_headers(router_for(root.path()), "/api/v1/repos/beta/stats").await;
    let etag = headers.get("etag").unwrap().to_str().unwrap().to_owned();

    let (status, _headers, body) = common::get_bytes_with_request_headers(
        router_for(root.path()),
        "/api/v1/repos/beta/stats",
        &[("if-none-match", &etag)],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert!(body.is_empty());
}
