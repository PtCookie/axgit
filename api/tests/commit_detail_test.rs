//! Integration tests for `GET /api/v1/repos/{repo}/commits/{sha}`.

mod common;

use std::path::Path;

use axum::http::StatusCode;
use serde_json::Value;
use tempfile::TempDir;

/// sha256 of `author@example.com` (the fixed fixture author email).
const AUTHOR_EMAIL_HASH: &str = "b0eda69977c26118feff17875d53376006568bcbcde5ca0c916d01f05c281436";
/// sha256 of `committer@example.com`.
const COMMITTER_EMAIL_HASH: &str =
    "f395b38df60e606322b6576159c903509f1a217b386580d8150965d33d8ef30f";

/// `alpha.git` covering the diffstat cases; shas oldest → newest:
/// 0. root: `a.txt` (2 lines) + `sub/nested.txt`
/// 1. modify `a.txt` (1 add / 1 del) + add `b.txt`, message has a body
/// 2. delete `b.txt`
/// 3. rename `a.txt` → `renamed.txt`
/// 4. add `bin.dat` (binary)
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

    std::fs::create_dir_all(work_path.join("sub")).unwrap();
    std::fs::write(work_path.join("a.txt"), "one\ntwo\n").unwrap();
    std::fs::write(work_path.join("sub/nested.txt"), "nested\n").unwrap();
    shas.push(commit_all(work_path, "feat: initial files"));

    std::fs::write(work_path.join("a.txt"), "one\nthree\n").unwrap();
    std::fs::write(work_path.join("b.txt"), "bee\n").unwrap();
    shas.push(commit_all(work_path, "fix: update a\n\nwith a body"));

    std::fs::remove_file(work_path.join("b.txt")).unwrap();
    shas.push(commit_all(work_path, "chore: drop b"));

    common::git(work_path, &["mv", "a.txt", "renamed.txt"]);
    shas.push(commit_all(work_path, "refactor: rename a"));

    std::fs::write(work_path.join("bin.dat"), b"\x00\x01\x02binary\x00").unwrap();
    shas.push(commit_all(work_path, "feat: add binary"));

    common::git(work_path, &["push", "--quiet", "origin", "HEAD:main"]);
    (root, shas)
}

use common::commit_all;
use common::router_for;

async fn get_ok(repo_root: &Path, uri: &str) -> Value {
    let (status, json) = common::get_json(router_for(repo_root), uri).await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");
    json
}

/// diffstat file entry for `path`, with a helpful panic when absent.
fn stat_file<'j>(json: &'j Value, path: &str) -> &'j Value {
    json["diffstat"]["files"]
        .as_array()
        .expect("diffstat.files missing")
        .iter()
        .find(|file| file["path"] == path)
        .unwrap_or_else(|| panic!("no diffstat entry for {path}: {json}"))
}

#[tokio::test]
async fn detail_should_return_full_message_and_signatures() {
    let (root, shas) = setup();
    let json = get_ok(
        root.path(),
        &format!("/api/v1/repos/alpha/commits/{}", shas[1]),
    )
    .await;
    assert_eq!(
        (
            json["sha"].as_str(),
            json["summary"].as_str(),
            json["message"].as_str(),
            json["author"]["name"].as_str(),
            json["author"]["email_hash"].as_str(),
            json["committer"]["name"].as_str(),
            json["committer"]["email_hash"].as_str(),
            json["authored_at"].as_str(),
            json["parents"][0].as_str(),
        ),
        (
            Some(shas[1].as_str()),
            Some("fix: update a"),
            Some("fix: update a\n\nwith a body\n"),
            Some("Test Author"),
            Some(AUTHOR_EMAIL_HASH),
            Some("Test Committer"),
            Some(COMMITTER_EMAIL_HASH),
            Some(common::FIXED_DATE),
            Some(shas[0].as_str()),
        ),
        "unexpected response: {json}"
    );
    // The raw email must never be exposed.
    assert!(json["author"].get("email").is_none());
}

#[tokio::test]
async fn detail_diffstat_should_report_modified_and_added_with_totals() {
    let (root, shas) = setup();
    let json = get_ok(
        root.path(),
        &format!("/api/v1/repos/alpha/commits/{}", shas[1]),
    )
    .await;
    let a = stat_file(&json, "a.txt");
    let b = stat_file(&json, "b.txt");
    assert_eq!(
        (
            json["diffstat"]["files_changed"].as_u64(),
            json["diffstat"]["total_additions"].as_u64(),
            json["diffstat"]["total_deletions"].as_u64(),
            a["status"].as_str(),
            a["additions"].as_u64(),
            a["deletions"].as_u64(),
            b["status"].as_str(),
            b["additions"].as_u64(),
        ),
        (
            Some(2),
            Some(2),
            Some(1),
            Some("modified"),
            Some(1),
            Some(1),
            Some("added"),
            Some(1),
        ),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn detail_diffstat_should_report_deleted() {
    let (root, shas) = setup();
    let json = get_ok(
        root.path(),
        &format!("/api/v1/repos/alpha/commits/{}", shas[2]),
    )
    .await;
    let b = stat_file(&json, "b.txt");
    assert_eq!(
        (b["status"].as_str(), b["deletions"].as_u64()),
        (Some("deleted"), Some(1)),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn detail_diffstat_should_report_renamed_with_old_path() {
    let (root, shas) = setup();
    let json = get_ok(
        root.path(),
        &format!("/api/v1/repos/alpha/commits/{}", shas[3]),
    )
    .await;
    let renamed = stat_file(&json, "renamed.txt");
    assert_eq!(
        (
            json["diffstat"]["files_changed"].as_u64(),
            renamed["status"].as_str(),
            renamed["old_path"].as_str(),
        ),
        (Some(1), Some("renamed"), Some("a.txt")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn detail_diffstat_should_flag_binary() {
    let (root, shas) = setup();
    let json = get_ok(
        root.path(),
        &format!("/api/v1/repos/alpha/commits/{}", shas[4]),
    )
    .await;
    let bin = stat_file(&json, "bin.dat");
    assert_eq!(
        (
            bin["binary"].as_bool(),
            bin["additions"].as_u64(),
            bin["deletions"].as_u64(),
        ),
        (Some(true), Some(0), Some(0)),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn detail_root_commit_should_diff_against_empty_tree() {
    let (root, shas) = setup();
    let json = get_ok(
        root.path(),
        &format!("/api/v1/repos/alpha/commits/{}", shas[0]),
    )
    .await;
    let a = stat_file(&json, "a.txt");
    let nested = stat_file(&json, "sub/nested.txt");
    assert_eq!(
        (
            json["parents"].as_array().map(Vec::len),
            a["status"].as_str(),
            nested["status"].as_str(),
        ),
        (Some(0), Some("added"), Some("added")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn detail_merge_should_diff_against_first_parent() {
    let root = tempfile::tempdir().unwrap();
    let bare = common::create_bare_repo(root.path(), "alpha.git");
    let work = tempfile::tempdir().unwrap();
    let work_path = work.path();
    common::git(
        work_path,
        &["clone", "--quiet", bare.to_str().unwrap(), "."],
    );
    std::fs::write(work_path.join("base.txt"), "base\n").unwrap();
    commit_all(work_path, "feat: base");
    common::git(work_path, &["checkout", "--quiet", "-b", "feature"]);
    std::fs::write(work_path.join("feature.txt"), "feature\n").unwrap();
    commit_all(work_path, "feat: feature file");
    common::git(work_path, &["checkout", "--quiet", "main"]);
    std::fs::write(work_path.join("main.txt"), "main\n").unwrap();
    commit_all(work_path, "feat: main file");
    common::git(
        work_path,
        &[
            "merge",
            "--quiet",
            "--no-ff",
            "-m",
            "merge feature",
            "feature",
        ],
    );
    let merge_sha = common::git_output(work_path, &["rev-parse", "HEAD"], &[]);
    common::git(work_path, &["push", "--quiet", "origin", "HEAD:main"]);

    let json = get_ok(
        root.path(),
        &format!("/api/v1/repos/alpha/commits/{merge_sha}"),
    )
    .await;
    // Against the first parent (main) only the feature branch's file appears.
    let feature = stat_file(&json, "feature.txt");
    assert_eq!(
        (
            json["parents"].as_array().map(Vec::len),
            json["diffstat"]["files_changed"].as_u64(),
            feature["status"].as_str(),
        ),
        (Some(2), Some(1), Some("added")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn detail_should_set_immutable_cache_for_full_sha() {
    let (root, shas) = setup();
    let (status, headers, json) = common::get_json_with_headers(
        router_for(root.path()),
        &format!("/api/v1/repos/alpha/commits/{}", shas[0]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");
    assert_eq!(
        headers
            .get("cache-control")
            .map(|value| value.to_str().unwrap()),
        Some("public, max-age=31536000, immutable")
    );
}

#[tokio::test]
async fn detail_should_not_cache_branch_addressed_requests() {
    let (root, shas) = setup();
    let (status, headers, json) =
        common::get_json_with_headers(router_for(root.path()), "/api/v1/repos/alpha/commits/main")
            .await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");
    // The branch resolves to the newest commit, but the response is mutable.
    assert_eq!(
        (json["sha"].as_str(), headers.get("cache-control")),
        (Some(shas[4].as_str()), None),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn detail_should_return_404_for_unknown_sha() {
    let (root, _shas) = setup();
    let (status, json) = common::get_json(
        router_for(root.path()),
        "/api/v1/repos/alpha/commits/no-such-ref",
    )
    .await;
    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("ref_not_found")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn detail_should_return_404_for_unknown_repo() {
    let root = tempfile::tempdir().unwrap();
    let (status, json) =
        common::get_json(router_for(root.path()), "/api/v1/repos/nope/commits/abc").await;
    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("repo_not_found")),
        "unexpected response: {json}"
    );
}
