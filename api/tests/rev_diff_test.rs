//! Integration tests for `GET /api/v1/repos/{repo}/diff`.

mod common;

use std::path::Path;

use axum::http::StatusCode;
use serde_json::Value;
use tempfile::TempDir;

use common::commit_all;
use common::router_for;

/// `alpha.git`, `main` branch; shas oldest → newest:
/// 0. root: `a.txt` (2 lines) + `sub/nested.txt`
/// 1. modify `a.txt` (`two` → `three`) + add `b.txt`
/// 2. rename `a.txt` → `renamed.txt`
/// 3. add `bin.dat` (binary)
///
/// Plus a `feature/x` branch off commit 1 with one extra commit adding
/// `feature.txt`, never merged into `main`.
fn setup() -> (TempDir, Vec<String>, String) {
    let root = tempfile::tempdir().expect("failed to create fixture root");
    let bare = common::create_bare_repo(root.path(), "alpha.git");
    let work = tempfile::tempdir().expect("failed to create work dir");
    let work_path = work.path();
    common::git(
        work_path,
        &["clone", "--quiet", bare.to_str().unwrap(), "."],
    );
    let mut shas = Vec::new();

    std::fs::create_dir_all(work_path.join("sub")).unwrap();
    std::fs::write(work_path.join("a.txt"), "one\ntwo\n").unwrap();
    std::fs::write(work_path.join("sub/nested.txt"), "nested\n").unwrap();
    shas.push(commit_all(work_path, "feat: initial files"));

    std::fs::write(work_path.join("a.txt"), "one\nthree\n").unwrap();
    std::fs::write(work_path.join("b.txt"), "bee\n").unwrap();
    shas.push(commit_all(work_path, "fix: update a"));

    common::git(work_path, &["branch", "--quiet", "feature/x"]);

    common::git(work_path, &["mv", "a.txt", "renamed.txt"]);
    shas.push(commit_all(work_path, "refactor: rename a"));

    std::fs::write(work_path.join("bin.dat"), b"\x00\x01\x02binary\x00").unwrap();
    shas.push(commit_all(work_path, "feat: add binary"));

    common::git(work_path, &["push", "--quiet", "origin", "HEAD:main"]);

    common::git(work_path, &["checkout", "--quiet", "feature/x"]);
    std::fs::write(work_path.join("feature.txt"), "feature\n").unwrap();
    let feature_sha = commit_all(work_path, "feat: feature file");
    common::git(work_path, &["push", "--quiet", "origin", "HEAD:feature/x"]);

    (root, shas, feature_sha)
}

async fn get_ok(repo_root: &Path, uri: &str) -> Value {
    let (status, json) = common::get_json(router_for(repo_root), uri).await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");
    json
}

fn file_entry<'j>(json: &'j Value, path: &str) -> &'j Value {
    json["files"]
        .as_array()
        .expect("files missing")
        .iter()
        .find(|file| file["path"] == path)
        .unwrap_or_else(|| panic!("no diff entry for {path}: {json}"))
}

#[tokio::test]
async fn diff_should_default_to_head_and_first_parent() {
    let (root, shas, _feature_sha) = setup();
    let json = get_ok(root.path(), "/api/v1/repos/alpha/diff").await;
    assert_eq!(
        (json["from"].as_str(), json["to"].as_str()),
        (Some(shas[2].as_str()), Some(shas[3].as_str())),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn diff_with_to_only_should_equal_the_commit_diff_endpoint() {
    let (root, shas, _feature_sha) = setup();
    let rev_diff_json = get_ok(
        root.path(),
        &format!("/api/v1/repos/alpha/diff?to={}", shas[1]),
    )
    .await;
    let commit_diff_json = get_ok(
        root.path(),
        &format!("/api/v1/repos/alpha/commits/{}/diff", shas[1]),
    )
    .await;
    assert_eq!(
        rev_diff_json["files"], commit_diff_json["files"],
        "rev_diff with to-only was not a superset of the commit diff: {rev_diff_json} vs {commit_diff_json}"
    );
    assert_eq!(rev_diff_json["from"], commit_diff_json["parent"]);
    assert_eq!(rev_diff_json["to"], commit_diff_json["sha"]);
}

#[tokio::test]
async fn diff_should_compare_two_arbitrary_revisions() {
    let (root, shas, _feature_sha) = setup();
    // shas[0] -> shas[3]: a.txt renamed+modified, b.txt added, bin.dat added.
    let json = get_ok(
        root.path(),
        &format!("/api/v1/repos/alpha/diff?from={}&to={}", shas[0], shas[3]),
    )
    .await;
    let paths: Vec<_> = json["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap().to_owned())
        .collect();
    assert!(
        paths.contains(&"renamed.txt".to_owned()),
        "unexpected files: {paths:?}"
    );
    assert!(
        paths.contains(&"b.txt".to_owned()),
        "unexpected files: {paths:?}"
    );
    assert!(
        paths.contains(&"bin.dat".to_owned()),
        "unexpected files: {paths:?}"
    );
}

#[tokio::test]
async fn diff_should_not_show_a_file_added_then_deleted_in_between() {
    let root = tempfile::tempdir().unwrap();
    let bare = common::create_bare_repo(root.path(), "alpha.git");
    let work = tempfile::tempdir().unwrap();
    let work_path = work.path();
    common::git(
        work_path,
        &["clone", "--quiet", bare.to_str().unwrap(), "."],
    );
    std::fs::write(work_path.join("base.txt"), "base\n").unwrap();
    let start = commit_all(work_path, "feat: base");
    std::fs::write(work_path.join("temp.txt"), "temp\n").unwrap();
    commit_all(work_path, "feat: add temp file");
    std::fs::remove_file(work_path.join("temp.txt")).unwrap();
    let end = commit_all(work_path, "feat: remove temp file");
    common::git(work_path, &["push", "--quiet", "origin", "HEAD:main"]);

    let json = get_ok(
        root.path(),
        &format!("/api/v1/repos/alpha/diff?from={start}&to={end}"),
    )
    .await;
    assert_eq!(
        json["files"].as_array().map(Vec::len),
        Some(0),
        "temp.txt should not appear in a diff spanning its whole lifetime: {json}"
    );
}

#[tokio::test]
async fn diff_should_accept_a_ref_containing_a_slash() {
    let (root, shas, feature_sha) = setup();
    let json = get_ok(
        root.path(),
        "/api/v1/repos/alpha/diff?from=main&to=feature%2Fx",
    )
    .await;
    assert_eq!(
        (json["from"].as_str(), json["to"].as_str()),
        (Some(shas[3].as_str()), Some(feature_sha.as_str())),
        "unexpected response: {json}"
    );
    let paths: Vec<_> = json["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap().to_owned())
        .collect();
    assert!(
        paths.contains(&"feature.txt".to_owned()),
        "unexpected files: {paths:?}"
    );
}

#[tokio::test]
async fn diff_should_report_from_null_for_a_root_commit() {
    let (root, shas, _feature_sha) = setup();
    let json = get_ok(
        root.path(),
        &format!("/api/v1/repos/alpha/diff?to={}", shas[0]),
    )
    .await;
    assert_eq!(
        json["from"].as_null(),
        Some(()),
        "unexpected response: {json}"
    );
    let a = file_entry(&json, "a.txt");
    assert_eq!(a["status"].as_str(), Some("added"));
}

#[tokio::test]
async fn diff_should_404_naming_the_offending_ref() {
    let (root, shas, _feature_sha) = setup();

    let (status, json) = common::get_json(
        router_for(root.path()),
        "/api/v1/repos/alpha/diff?from=no-such-ref",
    )
    .await;
    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("ref_not_found")),
        "unexpected response: {json}"
    );
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no-such-ref"),
        "error message did not name the offending ref: {json}"
    );

    let (status, json) = common::get_json(
        router_for(root.path()),
        &format!("/api/v1/repos/alpha/diff?from={}&to=no-such-ref", shas[0]),
    )
    .await;
    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("ref_not_found")),
        "unexpected response: {json}"
    );
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no-such-ref"),
        "error message did not name the offending ref: {json}"
    );
}

#[tokio::test]
async fn diff_should_apply_the_path_filter() {
    let (root, shas, _feature_sha) = setup();
    let json = get_ok(
        root.path(),
        &format!(
            "/api/v1/repos/alpha/diff?from={}&to={}&path=b.txt",
            shas[0], shas[3]
        ),
    )
    .await;
    assert_eq!(
        (
            json["files"].as_array().map(Vec::len),
            json["files"][0]["path"].as_str(),
        ),
        (Some(1), Some("b.txt")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn diff_unmatched_path_should_yield_empty_files() {
    let (root, shas, _feature_sha) = setup();
    let json = get_ok(
        root.path(),
        &format!(
            "/api/v1/repos/alpha/diff?from={}&to={}&path=no-such.txt",
            shas[0], shas[3]
        ),
    )
    .await;
    assert_eq!(
        json["files"].as_array().map(Vec::len),
        Some(0),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn diff_should_reverse_additions_and_deletions_when_swapped() {
    let (root, shas, _feature_sha) = setup();
    let forward = get_ok(
        root.path(),
        &format!("/api/v1/repos/alpha/diff?from={}&to={}", shas[0], shas[1]),
    )
    .await;
    let backward = get_ok(
        root.path(),
        &format!("/api/v1/repos/alpha/diff?from={}&to={}", shas[1], shas[0]),
    )
    .await;
    let forward_a = file_entry(&forward, "a.txt");
    let backward_a = file_entry(&backward, "a.txt");
    assert_eq!(
        (
            forward_a["additions"].as_u64(),
            forward_a["deletions"].as_u64()
        ),
        (
            backward_a["deletions"].as_u64(),
            backward_a["additions"].as_u64()
        ),
        "swapping from/to should mirror additions and deletions: {forward} vs {backward}"
    );
}

#[tokio::test]
async fn diff_should_be_immutable_only_when_every_given_side_is_a_full_sha() {
    let (root, shas, _feature_sha) = setup();

    // Both sides full shas: immutable, no ETag.
    let (status, headers, json) = common::get_json_with_headers(
        router_for(root.path()),
        &format!("/api/v1/repos/alpha/diff?from={}&to={}", shas[0], shas[1]),
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

    // `to` symbolic: mutable.
    let (status, headers, json) =
        common::get_json_with_headers(router_for(root.path()), "/api/v1/repos/alpha/diff?to=main")
            .await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");
    assert_eq!(
        headers
            .get("cache-control")
            .map(|value| value.to_str().unwrap()),
        Some("no-cache")
    );
    assert!(headers.contains_key("etag"));

    // `from` full sha, `to` symbolic: still mutable (to drives it).
    let (status, headers, json) = common::get_json_with_headers(
        router_for(root.path()),
        &format!("/api/v1/repos/alpha/diff?from={}&to=main", shas[0]),
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
async fn diff_should_return_304_on_if_none_match() {
    let (root, _shas, _feature_sha) = setup();
    let router = router_for(root.path());
    let (status, headers, json) =
        common::get_json_with_headers(router.clone(), "/api/v1/repos/alpha/diff?to=main").await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");
    let etag = headers
        .get("etag")
        .expect("expected an etag")
        .to_str()
        .unwrap()
        .to_owned();

    let (status, _headers, _body) = common::get_bytes_with_request_headers(
        router,
        "/api/v1/repos/alpha/diff?to=main",
        &[("if-none-match", etag.as_str())],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
}

#[tokio::test]
async fn diff_should_honor_context_and_ignorews() {
    let (root, shas, _feature_sha) = setup();
    let json = get_ok(
        root.path(),
        &format!(
            "/api/v1/repos/alpha/diff?from={}&to={}&context=0",
            shas[0], shas[1]
        ),
    )
    .await;
    let a = file_entry(&json, "a.txt");
    let origins: Vec<_> = a["hunks"][0]["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| line["origin"].as_str().unwrap().to_owned())
        .collect();
    assert!(
        !origins.contains(&" ".to_owned()),
        "context=0 should drop context lines: {json}"
    );
}

#[tokio::test]
async fn diff_should_return_404_for_unknown_repo() {
    let root = tempfile::tempdir().unwrap();
    let (status, json) = common::get_json(router_for(root.path()), "/api/v1/repos/nope/diff").await;
    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("repo_not_found")),
        "unexpected response: {json}"
    );
}
