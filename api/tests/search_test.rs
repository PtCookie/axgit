//! Integration tests for `GET /api/v1/repos/{repo}/search`.

mod common;

use std::path::Path;

use axum::http::StatusCode;
use serde_json::Value;
use tempfile::TempDir;

use common::commit_all;
use common::router_for;

/// `alpha.git`; shas oldest → newest:
/// 0. root: `src/main.rs` ("fn main() { println!(\"hello\"); }"), `README.md`
/// 1. `src/lib.rs` ("pub fn helper() {}"), commit message "fix: add a helper function"
/// 2. `bin.dat` (binary, contains the ASCII needle "findme" past a NUL byte)
fn setup() -> (TempDir, Vec<String>) {
    let root = tempfile::tempdir().expect("failed to create fixture root");
    let bare = common::create_bare_repo(root.path(), "alpha.git");
    let work = tempfile::tempdir().expect("failed to create work dir");
    let work_path = work.path();
    common::git(
        work_path,
        &["clone", "--quiet", bare.to_str().unwrap(), "."],
    );
    let mut shas = Vec::new();

    std::fs::create_dir_all(work_path.join("src")).unwrap();
    std::fs::write(
        work_path.join("src/main.rs"),
        "fn main() { println!(\"hello\"); }\n",
    )
    .unwrap();
    std::fs::write(work_path.join("README.md"), "# Alpha\n\nAn example repo.\n").unwrap();
    shas.push(commit_all(work_path, "feat: initial files"));

    std::fs::write(work_path.join("src/lib.rs"), "pub fn helper() {}\n").unwrap();
    shas.push(commit_all(work_path, "fix: add a helper function"));

    std::fs::write(
        work_path.join("bin.dat"),
        b"\x00\x01\x02findme-in-binary\x00",
    )
    .unwrap();
    shas.push(commit_all(work_path, "feat: add binary"));

    common::git(work_path, &["push", "--quiet", "origin", "HEAD:main"]);
    (root, shas)
}

async fn get_ok(root: &Path, uri: &str) -> Value {
    let (status, json) = common::get_json(router_for(root), uri).await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");
    json
}

fn file_paths(json: &Value) -> Vec<String> {
    json["files"]
        .as_array()
        .expect("files missing")
        .iter()
        .map(|file| file["path"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn search_content_finds_matching_lines_case_insensitively() {
    let (root, shas) = setup();
    let json = get_ok(root.path(), "/api/v1/repos/alpha/search?q=HELLO").await;
    assert_eq!(
        (
            json["sha"].as_str(),
            json["type"].as_str(),
            json["truncated"].as_bool(),
            file_paths(&json),
        ),
        (
            Some(shas[2].as_str()),
            Some("content"),
            Some(false),
            vec!["src/main.rs".to_owned()],
        ),
        "unexpected response: {json}"
    );
    let lines = &json["files"][0]["lines"];
    assert_eq!(lines[0]["line"], 1);
    assert_eq!(lines[0]["text"], "fn main() { println!(\"hello\"); }");
    assert_eq!(json["commits"], Value::Array(vec![]));
}

#[tokio::test]
async fn search_content_defaults_to_type_content() {
    let (root, _shas) = setup();
    let with_default = get_ok(root.path(), "/api/v1/repos/alpha/search?q=helper").await;
    let explicit = get_ok(
        root.path(),
        "/api/v1/repos/alpha/search?q=helper&type=content",
    )
    .await;
    assert_eq!(with_default["files"], explicit["files"]);
    assert_eq!(file_paths(&with_default), vec!["src/lib.rs".to_owned()]);
}

#[tokio::test]
async fn search_content_skips_binary_files() {
    let (root, _shas) = setup();
    let json = get_ok(root.path(), "/api/v1/repos/alpha/search?q=findme").await;
    assert_eq!(file_paths(&json), Vec::<String>::new());
}

#[tokio::test]
async fn search_path_matches_full_path_without_reading_content() {
    let (root, _shas) = setup();
    let json = get_ok(root.path(), "/api/v1/repos/alpha/search?q=src&type=path").await;
    let mut paths = file_paths(&json);
    paths.sort();
    assert_eq!(
        paths,
        vec!["src/lib.rs".to_owned(), "src/main.rs".to_owned()]
    );
    // Path matches don't carry line-level detail.
    assert_eq!(json["files"][0]["lines"], Value::Array(vec![]));
}

#[tokio::test]
async fn search_message_matches_commit_messages() {
    let (root, shas) = setup();
    let json = get_ok(
        root.path(),
        "/api/v1/repos/alpha/search?q=helper&type=message",
    )
    .await;
    assert_eq!(json["files"], Value::Array(vec![]));
    let commits = json["commits"].as_array().expect("commits missing");
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0]["sha"], shas[1]);
    assert_eq!(commits[0]["summary"], "fix: add a helper function");
}

#[tokio::test]
async fn search_author_matches_the_author_name_only() {
    // Every `alpha.git` commit is authored by "Test Author" and committed by
    // "Test Committer" (`api/tests/common/mod.rs`'s fixed env) — distinct
    // names, so `type=author` matching the committer's name would be a bug.
    let (root, shas) = setup();
    let json = get_ok(
        root.path(),
        "/api/v1/repos/alpha/search?q=Author&type=author",
    )
    .await;
    let commits = json["commits"].as_array().expect("commits missing");
    let matched: Vec<_> = commits
        .iter()
        .map(|c| c["sha"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(matched.len(), 3, "unexpected response: {json}");
    for sha in &shas {
        assert!(matched.contains(sha), "{sha} missing from {matched:?}");
    }
    assert_eq!(json["files"], Value::Array(vec![]));

    let json = get_ok(
        root.path(),
        "/api/v1/repos/alpha/search?q=Committer&type=author",
    )
    .await;
    assert_eq!(json["commits"], Value::Array(vec![]), "{json}");
}

#[tokio::test]
async fn search_committer_matches_the_committer_name_only() {
    let (root, shas) = setup();
    let json = get_ok(
        root.path(),
        "/api/v1/repos/alpha/search?q=Committer&type=committer",
    )
    .await;
    let commits = json["commits"].as_array().expect("commits missing");
    let matched: Vec<_> = commits
        .iter()
        .map(|c| c["sha"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(matched.len(), 3, "unexpected response: {json}");
    for sha in &shas {
        assert!(matched.contains(sha), "{sha} missing from {matched:?}");
    }

    let json = get_ok(
        root.path(),
        "/api/v1/repos/alpha/search?q=Author&type=committer",
    )
    .await;
    assert_eq!(json["commits"], Value::Array(vec![]), "{json}");
}

#[tokio::test]
async fn search_range_selects_commits_between_two_revisions() {
    let (root, shas) = setup();
    let json = get_ok(
        root.path(),
        &format!(
            "/api/v1/repos/alpha/search?type=range&q={}..{}",
            shas[0], shas[2]
        ),
    )
    .await;
    let commits = json["commits"].as_array().expect("commits missing");
    let matched: Vec<_> = commits
        .iter()
        .map(|c| c["sha"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(matched.len(), 2, "unexpected response: {json}");
    assert!(matched.contains(&shas[1]));
    assert!(matched.contains(&shas[2]));
    assert!(!matched.contains(&shas[0]));
    assert_eq!(json["files"], Value::Array(vec![]));
}

#[tokio::test]
async fn search_range_accepts_a_bare_revision() {
    let (root, shas) = setup();
    let json = get_ok(root.path(), "/api/v1/repos/alpha/search?type=range&q=main").await;
    let commits = json["commits"].as_array().expect("commits missing");
    assert_eq!(commits.len(), 3, "unexpected response: {json}");
    let matched: Vec<_> = commits
        .iter()
        .map(|c| c["sha"].as_str().unwrap().to_owned())
        .collect();
    for sha in &shas {
        assert!(matched.contains(sha));
    }
}

#[tokio::test]
async fn search_range_rejects_a_flag_looking_token() {
    let (root, _shas) = setup();
    let (status, json) = common::get_json(
        router_for(root.path()),
        "/api/v1/repos/alpha/search?type=range&q=--all",
    )
    .await;
    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("invalid_param")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn search_range_returns_404_for_an_unresolvable_revision() {
    let (root, _shas) = setup();
    let (status, json) = common::get_json(
        router_for(root.path()),
        "/api/v1/repos/alpha/search?type=range&q=no-such-rev..main",
    )
    .await;
    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("ref_not_found")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn search_range_is_never_immutable_even_for_a_full_sha_ref() {
    let (root, shas) = setup();
    let (status, headers, json) = common::get_json_with_headers(
        router_for(root.path()),
        &format!(
            "/api/v1/repos/alpha/search?type=range&q=main&ref={}",
            shas[2]
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");
    assert_eq!(
        headers
            .get("cache-control")
            .map(|value| value.to_str().unwrap()),
        Some("no-cache"),
        "type=range must never be immutably cached, even with a full-sha ref"
    );
    assert!(headers.contains_key("etag"));
}

#[tokio::test]
async fn search_rejects_missing_or_oversized_query() {
    let (root, _shas) = setup();
    for uri in [
        "/api/v1/repos/alpha/search",
        "/api/v1/repos/alpha/search?q=",
        "/api/v1/repos/alpha/search?q=%20%20",
        &format!("/api/v1/repos/alpha/search?q={}", "a".repeat(201)),
    ] {
        let (status, json) = common::get_json(router_for(root.path()), uri).await;
        assert_eq!(
            (status, json["error"]["code"].as_str()),
            (StatusCode::BAD_REQUEST, Some("invalid_param")),
            "uri {uri} was not rejected: {json}"
        );
    }
}

#[tokio::test]
async fn search_rejects_unknown_type() {
    let (root, _shas) = setup();
    let (status, json) = common::get_json(
        router_for(root.path()),
        "/api/v1/repos/alpha/search?q=hello&type=bogus",
    )
    .await;
    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("invalid_param")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn search_rejects_invalid_limit() {
    let (root, _shas) = setup();
    for limit in ["0", "101", "abc"] {
        let uri = format!("/api/v1/repos/alpha/search?q=hello&limit={limit}");
        let (status, json) = common::get_json(router_for(root.path()), &uri).await;
        assert_eq!(
            (status, json["error"]["code"].as_str()),
            (StatusCode::BAD_REQUEST, Some("invalid_param")),
            "limit={limit} was not rejected: {json}"
        );
    }
}

#[tokio::test]
async fn search_returns_404_for_unknown_ref() {
    let (root, _shas) = setup();
    let (status, json) = common::get_json(
        router_for(root.path()),
        "/api/v1/repos/alpha/search?q=hello&ref=no-such-ref",
    )
    .await;
    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("ref_not_found")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn search_returns_404_for_unknown_repo() {
    let root = tempfile::tempdir().unwrap();
    let (status, json) =
        common::get_json(router_for(root.path()), "/api/v1/repos/nope/search?q=hello").await;
    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("repo_not_found")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn search_returns_empty_result_for_empty_repo() {
    let root = tempfile::tempdir().expect("failed to create fixture root");
    common::create_bare_repo(root.path(), "empty.git");

    let json = get_ok(root.path(), "/api/v1/repos/empty/search?q=hello").await;
    assert_eq!(
        (
            json["sha"].as_null(),
            json["truncated"].as_bool(),
            json["files"].as_array().map(Vec::len),
            json["commits"].as_array().map(Vec::len),
        ),
        (Some(()), Some(false), Some(0), Some(0)),
        "unexpected response: {json}"
    );

    // An explicit ref on an empty repo is a resolution failure, not an empty result.
    let (status, json) = common::get_json(
        router_for(root.path()),
        "/api/v1/repos/empty/search?q=hello&ref=main",
    )
    .await;
    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("ref_not_found")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn search_applies_limit_and_reports_truncation() {
    let (root, _shas) = setup();
    let json = get_ok(
        root.path(),
        "/api/v1/repos/alpha/search?q=src&type=path&limit=1",
    )
    .await;
    assert_eq!(json["files"].as_array().map(Vec::len), Some(1));
    assert_eq!(json["truncated"].as_bool(), Some(true));
}

#[tokio::test]
async fn search_sets_immutable_cache_for_full_sha_ref_only() {
    let (root, shas) = setup();
    let (status, headers, json) = common::get_json_with_headers(
        router_for(root.path()),
        &format!("/api/v1/repos/alpha/search?q=hello&ref={}", shas[2]),
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

    let (status, headers, json) = common::get_json_with_headers(
        router_for(root.path()),
        "/api/v1/repos/alpha/search?q=hello&ref=main",
    )
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
async fn search_honors_if_none_match() {
    let (root, _shas) = setup();
    let (_, headers, _) = common::get_json_with_headers(
        router_for(root.path()),
        "/api/v1/repos/alpha/search?q=hello",
    )
    .await;
    let etag = headers.get("etag").unwrap().to_str().unwrap().to_owned();

    let (status, _headers, body) = common::get_bytes_with_request_headers(
        router_for(root.path()),
        "/api/v1/repos/alpha/search?q=hello",
        &[("if-none-match", &etag)],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert!(body.is_empty());
}
