//! Integration tests for `GET /api/v1/repos/{repo}/commits/{sha}/diff`.

mod common;

use std::path::Path;

use axum::http::StatusCode;
use serde_json::Value;
use tempfile::TempDir;

use common::commit_all;
use common::router_for;

/// `alpha.git`; shas oldest → newest:
/// 0. root: `a.txt` (2 lines) + `sub/nested.txt`
/// 1. modify `a.txt` (`two` → `three`) + add `b.txt`
/// 2. rename `a.txt` → `renamed.txt`
/// 3. add `bin.dat` (binary)
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
    shas.push(commit_all(work_path, "fix: update a"));

    common::git(work_path, &["mv", "a.txt", "renamed.txt"]);
    shas.push(commit_all(work_path, "refactor: rename a"));

    std::fs::write(work_path.join("bin.dat"), b"\x00\x01\x02binary\x00").unwrap();
    shas.push(commit_all(work_path, "feat: add binary"));

    common::git(work_path, &["push", "--quiet", "origin", "HEAD:main"]);
    (root, shas)
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
async fn diff_should_structure_hunks_and_lines() {
    let (root, shas) = setup();
    let json = get_ok(
        root.path(),
        &format!("/api/v1/repos/alpha/commits/{}/diff", shas[1]),
    )
    .await;
    let a = file_entry(&json, "a.txt");
    let hunk = &a["hunks"][0];
    assert_eq!(
        (
            json["sha"].as_str(),
            json["parent"].as_str(),
            json["truncated"].as_bool(),
            a["status"].as_str(),
            a["truncated"].as_bool(),
            hunk["header"].as_str(),
            hunk["old_start"].as_u64(),
            hunk["new_start"].as_u64(),
        ),
        (
            Some(shas[1].as_str()),
            Some(shas[0].as_str()),
            Some(false),
            Some("modified"),
            Some(false),
            Some("@@ -1,2 +1,2 @@"),
            Some(1),
            Some(1),
        ),
        "unexpected response: {json}"
    );
    let lines: Vec<_> = hunk["lines"]
        .as_array()
        .expect("lines missing")
        .iter()
        .map(|line| {
            (
                line["origin"].as_str().unwrap().to_owned(),
                line["content"].as_str().unwrap().to_owned(),
                line["old_lineno"].as_u64(),
                line["new_lineno"].as_u64(),
            )
        })
        .collect();
    assert_eq!(
        lines,
        vec![
            (" ".to_owned(), "one".to_owned(), Some(1), Some(1)),
            ("-".to_owned(), "two".to_owned(), Some(2), None),
            ("+".to_owned(), "three".to_owned(), None, Some(2)),
        ],
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn diff_root_commit_should_have_null_parent_and_all_additions() {
    let (root, shas) = setup();
    let json = get_ok(
        root.path(),
        &format!("/api/v1/repos/alpha/commits/{}/diff", shas[0]),
    )
    .await;
    let a = file_entry(&json, "a.txt");
    let origins: Vec<_> = a["hunks"][0]["lines"]
        .as_array()
        .expect("lines missing")
        .iter()
        .map(|line| line["origin"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        (json["parent"].as_null(), a["status"].as_str(), origins),
        (
            Some(()),
            Some("added"),
            vec!["+".to_owned(), "+".to_owned()]
        ),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn diff_should_flag_binary_with_empty_hunks() {
    let (root, shas) = setup();
    let json = get_ok(
        root.path(),
        &format!("/api/v1/repos/alpha/commits/{}/diff", shas[3]),
    )
    .await;
    let bin = file_entry(&json, "bin.dat");
    assert_eq!(
        (
            bin["binary"].as_bool(),
            bin["hunks"].as_array().map(Vec::len),
            bin["truncated"].as_bool(),
        ),
        (Some(true), Some(0), Some(false)),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn diff_rename_should_report_old_path_like_diffstat() {
    let (root, shas) = setup();
    let json = get_ok(
        root.path(),
        &format!("/api/v1/repos/alpha/commits/{}/diff", shas[2]),
    )
    .await;
    let renamed = file_entry(&json, "renamed.txt");
    assert_eq!(
        (
            json["files"].as_array().map(Vec::len),
            renamed["status"].as_str(),
            renamed["old_path"].as_str(),
            renamed["additions"].as_u64(),
            renamed["deletions"].as_u64(),
        ),
        (Some(1), Some("renamed"), Some("a.txt"), Some(0), Some(0)),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn diff_should_truncate_oversized_files_but_keep_full_stats() {
    let root = tempfile::tempdir().unwrap();
    let bare = common::create_bare_repo(root.path(), "alpha.git");
    let work = tempfile::tempdir().unwrap();
    let work_path = work.path();
    common::git(
        work_path,
        &["clone", "--quiet", bare.to_str().unwrap(), "."],
    );
    let big: String = (0..1100).map(|i| format!("line {i}\n")).collect();
    std::fs::write(work_path.join("big.txt"), big).unwrap();
    let sha = commit_all(work_path, "feat: add big file");
    common::git(work_path, &["push", "--quiet", "origin", "HEAD:main"]);

    let json = get_ok(
        root.path(),
        &format!("/api/v1/repos/alpha/commits/{sha}/diff"),
    )
    .await;
    let big = file_entry(&json, "big.txt");
    let rendered: usize = big["hunks"]
        .as_array()
        .expect("hunks missing")
        .iter()
        .map(|hunk| hunk["lines"].as_array().map_or(0, Vec::len))
        .sum();
    assert_eq!(
        (
            big["truncated"].as_bool(),
            big["additions"].as_u64(),
            json["truncated"].as_bool(),
        ),
        (Some(true), Some(1100), Some(false)),
        "unexpected response: {json}"
    );
    assert!(rendered <= 1000, "rendered {rendered} lines: {json}");
}

#[tokio::test]
async fn diff_merge_should_use_first_parent() {
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
    let first_parent = common::git_output(work_path, &["rev-parse", "HEAD^1"], &[]);
    common::git(work_path, &["push", "--quiet", "origin", "HEAD:main"]);

    let json = get_ok(
        root.path(),
        &format!("/api/v1/repos/alpha/commits/{merge_sha}/diff"),
    )
    .await;
    let feature = file_entry(&json, "feature.txt");
    assert_eq!(
        (
            json["parent"].as_str(),
            json["files"].as_array().map(Vec::len),
            feature["status"].as_str(),
        ),
        (Some(first_parent.as_str()), Some(1), Some("added")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn diff_path_should_limit_to_single_file() {
    let (root, shas) = setup();
    let json = get_ok(
        root.path(),
        &format!("/api/v1/repos/alpha/commits/{}/diff?path=a.txt", shas[1]),
    )
    .await;
    assert_eq!(
        (
            json["files"].as_array().map(Vec::len),
            json["files"][0]["path"].as_str(),
        ),
        (Some(1), Some("a.txt")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn diff_path_should_return_empty_for_missing_path() {
    let (root, shas) = setup();
    let json = get_ok(
        root.path(),
        &format!(
            "/api/v1/repos/alpha/commits/{}/diff?path=no-such.txt",
            shas[1]
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
async fn diff_path_should_return_empty_for_untouched_path() {
    let (root, shas) = setup();
    // sub/nested.txt exists but commit 1 did not touch it.
    let json = get_ok(
        root.path(),
        &format!(
            "/api/v1/repos/alpha/commits/{}/diff?path=sub/nested.txt",
            shas[1]
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
async fn diff_should_set_immutable_cache_for_full_sha_only() {
    let (root, shas) = setup();
    let (status, headers, json) = common::get_json_with_headers(
        router_for(root.path()),
        &format!("/api/v1/repos/alpha/commits/{}/diff", shas[0]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");
    assert_eq!(
        headers
            .get("cache-control")
            .map(|value| value.to_str().unwrap()),
        Some("public, max-age=31536000, immutable")
    );

    let (status, headers, json) = common::get_json_with_headers(
        router_for(root.path()),
        "/api/v1/repos/alpha/commits/main/diff",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");
    // Branch-addressed: mutable, so an ETag + no-cache instead.
    assert_eq!(
        headers
            .get("cache-control")
            .map(|value| value.to_str().unwrap()),
        Some("no-cache")
    );
    assert!(headers.contains_key("etag"));
}

#[tokio::test]
async fn diff_should_return_404_for_unknown_sha() {
    let (root, _shas) = setup();
    let (status, json) = common::get_json(
        router_for(root.path()),
        "/api/v1/repos/alpha/commits/no-such-ref/diff",
    )
    .await;
    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("ref_not_found")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn context_should_widen_hunks_when_requested() {
    let root = tempfile::tempdir().unwrap();
    let bare = common::create_bare_repo(root.path(), "alpha.git");
    let work = tempfile::tempdir().unwrap();
    let work_path = work.path();
    common::git(
        work_path,
        &["clone", "--quiet", bare.to_str().unwrap(), "."],
    );
    let mut lines: Vec<String> = (0..20).map(|i| format!("line {i}\n")).collect();
    std::fs::write(work_path.join("a.txt"), lines.join("")).unwrap();
    commit_all(work_path, "feat: base");
    lines[2] = "line 2 changed\n".to_owned();
    lines[11] = "line 11 changed\n".to_owned();
    std::fs::write(work_path.join("a.txt"), lines.join("")).unwrap();
    let sha = commit_all(work_path, "fix: two changes");
    common::git(work_path, &["push", "--quiet", "origin", "HEAD:main"]);

    let default_json = get_ok(
        root.path(),
        &format!("/api/v1/repos/alpha/commits/{sha}/diff"),
    )
    .await;
    assert_eq!(
        default_json["files"][0]["hunks"].as_array().map(Vec::len),
        Some(2),
        "unexpected response: {default_json}"
    );

    let wide_json = get_ok(
        root.path(),
        &format!("/api/v1/repos/alpha/commits/{sha}/diff?context=10"),
    )
    .await;
    assert_eq!(
        wide_json["files"][0]["hunks"].as_array().map(Vec::len),
        Some(1),
        "unexpected response: {wide_json}"
    );
}

#[tokio::test]
async fn context_should_reject_out_of_range_and_non_numeric_values() {
    let (root, shas) = setup();
    for bad in ["101", "-1", "abc", ""] {
        let (status, json) = common::get_json(
            router_for(root.path()),
            &format!("/api/v1/repos/alpha/commits/{}/diff?context={bad}", shas[1]),
        )
        .await;
        assert_eq!(
            (status, json["error"]["code"].as_str()),
            (StatusCode::BAD_REQUEST, Some("invalid_param")),
            "context={bad:?} unexpectedly accepted: {json}"
        );
    }
}

#[tokio::test]
async fn ignorews_should_hide_whitespace_only_changes() {
    let root = tempfile::tempdir().unwrap();
    let bare = common::create_bare_repo(root.path(), "alpha.git");
    let work = tempfile::tempdir().unwrap();
    let work_path = work.path();
    common::git(
        work_path,
        &["clone", "--quiet", bare.to_str().unwrap(), "."],
    );
    std::fs::write(work_path.join("a.txt"), "one\ntwo\n").unwrap();
    commit_all(work_path, "feat: base");
    std::fs::write(work_path.join("a.txt"), "one\ntwo  \n").unwrap();
    let sha = commit_all(work_path, "style: trailing whitespace");
    common::git(work_path, &["push", "--quiet", "origin", "HEAD:main"]);

    let default_json = get_ok(
        root.path(),
        &format!("/api/v1/repos/alpha/commits/{sha}/diff"),
    )
    .await;
    assert!(
        !default_json["files"][0]["hunks"]
            .as_array()
            .unwrap()
            .is_empty(),
        "expected the whitespace change to be visible by default: {default_json}"
    );

    let ignorews_json = get_ok(
        root.path(),
        &format!("/api/v1/repos/alpha/commits/{sha}/diff?ignorews=1"),
    )
    .await;
    let file = &ignorews_json["files"][0];
    assert_eq!(
        (
            file["hunks"].as_array().map(Vec::len),
            file["additions"].as_u64(),
            file["deletions"].as_u64(),
        ),
        (Some(0), Some(0), Some(0)),
        "unexpected response: {ignorews_json}"
    );
}

#[tokio::test]
async fn ignorews_should_reject_bad_values() {
    let (root, shas) = setup();
    let (status, json) = common::get_json(
        router_for(root.path()),
        &format!("/api/v1/repos/alpha/commits/{}/diff?ignorews=yes", shas[1]),
    )
    .await;
    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("invalid_param")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn diff_options_should_be_part_of_the_cache_key() {
    let root = tempfile::tempdir().unwrap();
    let bare = common::create_bare_repo(root.path(), "alpha.git");
    let work = tempfile::tempdir().unwrap();
    let work_path = work.path();
    common::git(
        work_path,
        &["clone", "--quiet", bare.to_str().unwrap(), "."],
    );
    let mut lines: Vec<String> = (0..20).map(|i| format!("line {i}\n")).collect();
    std::fs::write(work_path.join("a.txt"), lines.join("")).unwrap();
    commit_all(work_path, "feat: base");
    lines[2] = "line 2 changed\n".to_owned();
    lines[11] = "line 11 changed\n".to_owned();
    std::fs::write(work_path.join("a.txt"), lines.join("")).unwrap();
    let sha = commit_all(work_path, "fix: two changes");
    common::git(work_path, &["push", "--quiet", "origin", "HEAD:main"]);

    // One shared router so the two requests hit the same in-process cache.
    let router = router_for(root.path());
    let (status_a, json_a) = common::get_json(
        router.clone(),
        &format!("/api/v1/repos/alpha/commits/{sha}/diff?context=3"),
    )
    .await;
    let (status_b, json_b) = common::get_json(
        router,
        &format!("/api/v1/repos/alpha/commits/{sha}/diff?context=10"),
    )
    .await;
    assert_eq!(status_a, StatusCode::OK, "unexpected response: {json_a}");
    assert_eq!(status_b, StatusCode::OK, "unexpected response: {json_b}");
    assert_ne!(
        json_a, json_b,
        "context=3 and context=10 unexpectedly hit the same cache entry"
    );
}

#[tokio::test]
async fn diff_should_return_404_for_unknown_repo() {
    let root = tempfile::tempdir().unwrap();
    let (status, json) = common::get_json(
        router_for(root.path()),
        "/api/v1/repos/nope/commits/abc/diff",
    )
    .await;
    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("repo_not_found")),
        "unexpected response: {json}"
    );
}
