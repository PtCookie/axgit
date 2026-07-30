//! Integration tests for `GET /api/v1/repos/{repo}/blame/{ref}/{path...}`.

mod common;

use std::path::Path;

use axum::http::{StatusCode, header};
use serde_json::Value;
use tempfile::TempDir;

use common::CommitSpec;
use common::router_for;

/// sha256 of `author@example.com` (the fixed fixture author email).
const AUTHOR_EMAIL_HASH: &str = "b0eda69977c26118feff17875d53376006568bcbcde5ca0c916d01f05c281436";

const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// `alpha.git` on `main`: `a.txt` edited across 3 commits (shas oldest → newest).
/// Line 1 is untouched after the first commit, line 2 is rewritten by the
/// second, line 3 is added by the third.
fn setup_history() -> (TempDir, Vec<String>) {
    let root = tempfile::tempdir().expect("failed to create fixture root");
    let alpha = common::create_bare_repo(root.path(), "alpha.git");
    let shas = common::commit_history(
        &alpha,
        &[
            CommitSpec {
                file: "a.txt",
                content: "one\ntwo\n",
                message: "feat: add a",
                date: "2026-07-01T12:00:00+09:00",
            },
            CommitSpec {
                file: "a.txt",
                content: "one\nTWO\n",
                message: "fix: update a",
                date: "2026-07-01T13:00:00+09:00",
            },
            CommitSpec {
                file: "a.txt",
                content: "one\nTWO\nthree\n",
                message: "feat: extend a",
                date: "2026-07-01T14:00:00+09:00",
            },
        ],
    );
    (root, shas)
}

async fn get_ok(root: &Path, uri: &str) -> Value {
    let (status, json) = common::get_json(router_for(root), uri).await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");
    json
}

async fn assert_error(root: &Path, uri: &str, status: StatusCode, code: &str) {
    let (actual, json) = common::get_json(router_for(root), uri).await;
    assert_eq!(actual, status, "unexpected response: {json}");
    assert_eq!(json["error"]["code"], code, "unexpected body: {json}");
}

#[tokio::test]
async fn blame_should_split_ranges_by_commit_with_metadata() {
    let (root, shas) = setup_history();
    let [root_sha, second_sha, third_sha] = [&shas[0], &shas[1], &shas[2]];

    let body = get_ok(root.path(), "/api/v1/repos/alpha/blame/main/a.txt").await;
    assert_eq!(body["sha"], *third_sha);
    assert_eq!(body["path"], "a.txt");
    assert_eq!(body["binary"], false);
    assert_eq!(body["too_large"], false);
    assert_eq!(body["lines"], 3);

    let ranges = body["ranges"].as_array().unwrap();
    assert_eq!(ranges.len(), 3, "unexpected ranges: {ranges:?}");

    assert_eq!(ranges[0]["start_line"], 1);
    assert_eq!(ranges[0]["line_count"], 1);
    assert_eq!(ranges[0]["sha"], *root_sha);
    assert_eq!(ranges[0]["summary"], "feat: add a");
    assert_eq!(ranges[0]["author"]["name"], "Test Author");
    assert_eq!(ranges[0]["author"]["email_hash"], AUTHOR_EMAIL_HASH);
    assert_eq!(ranges[0]["authored_at"], "2026-07-01T12:00:00+09:00");

    assert_eq!(ranges[1]["start_line"], 2);
    assert_eq!(ranges[1]["line_count"], 1);
    assert_eq!(ranges[1]["sha"], *second_sha);
    assert_eq!(ranges[1]["summary"], "fix: update a");
    assert_eq!(ranges[1]["authored_at"], "2026-07-01T13:00:00+09:00");

    assert_eq!(ranges[2]["start_line"], 3);
    assert_eq!(ranges[2]["line_count"], 1);
    assert_eq!(ranges[2]["sha"], *third_sha);
    assert_eq!(ranges[2]["summary"], "feat: extend a");
    assert_eq!(ranges[2]["authored_at"], "2026-07-01T14:00:00+09:00");
}

#[tokio::test]
async fn blame_should_resolve_branch_names_containing_slashes() {
    let (root, shas) = setup_history();
    let bare = root.path().join("alpha.git");
    common::add_branch(&bare, "feature/x");

    let body = get_ok(root.path(), "/api/v1/repos/alpha/blame/feature/x/a.txt").await;
    assert_eq!(body["sha"], shas[2]);
    assert_eq!(body["path"], "a.txt");
    assert_eq!(body["ranges"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn blame_should_flag_binary_and_oversized_files_with_empty_ranges() {
    let root = tempfile::tempdir().unwrap();
    let bare = common::create_bare_repo(root.path(), "files.git");
    let work = tempfile::tempdir().unwrap();
    common::git(
        work.path(),
        &["clone", "--quiet", bare.to_str().unwrap(), "."],
    );
    std::fs::write(work.path().join("logo.png"), b"\x89PNG\r\n\x1a\n\x00binary").unwrap();
    std::fs::write(work.path().join("big.txt"), "a".repeat(1024 * 1024 + 1)).unwrap();
    std::fs::write(work.path().join("empty.txt"), "").unwrap();
    common::commit_all(work.path(), "feat: files");
    common::git(work.path(), &["push", "--quiet", "origin", "HEAD:main"]);

    let body = get_ok(root.path(), "/api/v1/repos/files/blame/main/logo.png").await;
    assert_eq!(body["binary"], true);
    assert_eq!(body["too_large"], false);
    assert_eq!(body["lines"], 0);
    assert_eq!(body["ranges"], Value::Array(Vec::new()));

    let body = get_ok(root.path(), "/api/v1/repos/files/blame/main/big.txt").await;
    assert_eq!(body["binary"], false);
    assert_eq!(body["too_large"], true);
    assert_eq!(body["lines"], 0);
    assert_eq!(body["ranges"], Value::Array(Vec::new()));

    let body = get_ok(root.path(), "/api/v1/repos/files/blame/main/empty.txt").await;
    assert_eq!(body["binary"], false);
    assert_eq!(body["too_large"], false);
    assert_eq!(body["lines"], 0);
    assert_eq!(body["ranges"], Value::Array(Vec::new()));
}

#[tokio::test]
async fn blame_should_return_404_for_missing_path_or_directory() {
    let (root, _shas) = setup_history();
    for uri in [
        "/api/v1/repos/alpha/blame/main/missing.txt",
        "/api/v1/repos/alpha/blame/main",
    ] {
        assert_error(root.path(), uri, StatusCode::NOT_FOUND, "path_not_found").await;
    }
    assert_error(
        root.path(),
        "/api/v1/repos/alpha/blame/no-such-branch/a.txt",
        StatusCode::NOT_FOUND,
        "ref_not_found",
    )
    .await;
}

#[tokio::test]
async fn blame_should_reject_dot_segments() {
    let (root, _shas) = setup_history();
    assert_error(
        root.path(),
        "/api/v1/repos/alpha/blame/main/../../etc",
        StatusCode::BAD_REQUEST,
        "invalid_param",
    )
    .await;
}

#[tokio::test]
async fn blame_should_use_immutable_cache_control_only_for_full_sha() {
    let (root, shas) = setup_history();
    let head = &shas[2];
    let router = router_for(root.path());

    let (status, headers, body) = common::get_json_with_headers(
        router.clone(),
        &format!("/api/v1/repos/alpha/blame/{head}/a.txt"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {body}");
    assert_eq!(headers[header::CACHE_CONTROL], IMMUTABLE);
    assert!(!headers.contains_key(header::ETAG));

    let (status, headers, body) =
        common::get_json_with_headers(router.clone(), "/api/v1/repos/alpha/blame/main/a.txt").await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {body}");
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
    let etag = headers[header::ETAG].to_str().unwrap().to_owned();

    let (status, headers, body) = common::get_bytes_with_request_headers(
        router,
        "/api/v1/repos/alpha/blame/main/a.txt",
        &[("if-none-match", &etag)],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert!(body.is_empty());
    assert_eq!(headers[header::ETAG].to_str().unwrap(), etag);
}
