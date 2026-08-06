//! Integration tests for the tree/blob/raw/readme endpoints.

mod common;

use common::router_for;

use std::path::Path;

use axum::http::{StatusCode, header};
use serde_json::{Value, json};
use tempfile::TempDir;

/// Any 40-hex sha works for a gitlink; the object need not exist locally.
const GITLINK_SHA: &str = "0123456789abcdef0123456789abcdef01234567";

const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// `files.git` on `main` (returned sha #0):
///   README.md, noext, big.txt (1 MiB + 1), link -> README.md (symlink),
///   src/main.rs, src/lib/util.rs, src/readme-link -> ../README.md (symlink
///   with a target relative to its own directory), assets/logo.png (binary),
///   feature/x/inside.txt (directory shadowing the branch name),
///   vendor/dep (gitlink).
/// Branch `feature/x` (returned sha #1) adds branch-file.txt on top.
fn setup_files_repo(root: &Path) -> (String, String) {
    let bare = common::create_bare_repo(root, "files.git");
    let work = tempfile::tempdir().expect("failed to create work dir");
    let w = work.path();
    common::git(w, &["clone", "--quiet", bare.to_str().unwrap(), "."]);

    std::fs::create_dir_all(w.join("src/lib")).unwrap();
    std::fs::create_dir_all(w.join("assets")).unwrap();
    std::fs::create_dir_all(w.join("feature/x")).unwrap();
    // Empty directory keeps `git add -A` from staging the gitlink's removal.
    std::fs::create_dir_all(w.join("vendor/dep")).unwrap();
    std::fs::write(w.join("README.md"), "# Files fixture\n").unwrap();
    std::fs::write(w.join("noext"), "plain text\n").unwrap();
    std::fs::write(w.join("big.txt"), "a".repeat(1024 * 1024 + 1)).unwrap();
    std::fs::write(w.join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(w.join("src/lib/util.rs"), "pub fn util() {}\n").unwrap();
    std::fs::write(w.join("assets/logo.png"), b"\x89PNG\r\n\x1a\n\x00binary").unwrap();
    std::fs::write(w.join("feature/x/inside.txt"), "from main\n").unwrap();
    std::os::unix::fs::symlink("README.md", w.join("link")).unwrap();
    std::os::unix::fs::symlink("../README.md", w.join("src/readme-link")).unwrap();
    common::git(w, &["add", "-A"]);
    common::git(
        w,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{GITLINK_SHA},vendor/dep"),
        ],
    );
    common::git(w, &["commit", "--quiet", "-m", "feat: initial files"]);
    let head = common::git_output(w, &["rev-parse", "HEAD"], &[]);
    common::git(w, &["push", "--quiet", "origin", "HEAD:main"]);

    common::git(w, &["checkout", "--quiet", "-b", "feature/x"]);
    std::fs::write(w.join("branch-file.txt"), "on branch\n").unwrap();
    let branch = common::commit_all(w, "feat: branch file");
    common::git(w, &["push", "--quiet", "origin", "feature/x"]);

    (head, branch)
}

fn setup() -> (TempDir, String, String) {
    let root = tempfile::tempdir().expect("failed to create fixture root");
    let (head, branch) = setup_files_repo(root.path());
    (root, head, branch)
}

async fn get_ok(repo_root: &Path, uri: &str) -> Value {
    let (status, json) = common::get_json(router_for(repo_root), uri).await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");
    json
}

async fn assert_error(repo_root: &Path, uri: &str, status: StatusCode, code: &str) {
    let (actual, json) = common::get_json(router_for(repo_root), uri).await;
    assert_eq!(actual, status, "unexpected response: {json}");
    assert_eq!(json["error"]["code"], code, "unexpected body: {json}");
}

// --- tree ---

#[tokio::test]
async fn tree_should_list_root_with_trees_first() {
    let (root, head, _) = setup();
    let body = get_ok(root.path(), "/api/v1/repos/files/tree/main").await;
    assert_eq!(body["sha"], head);
    assert_eq!(body["path"], "");
    let names: Vec<&str> = body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "assets",
            "feature",
            "src",
            "vendor",
            "README.md",
            "big.txt",
            "link",
            "noext"
        ]
    );
    let entries = body["entries"].as_array().unwrap();
    let by_name = |name: &str| {
        entries
            .iter()
            .find(|entry| entry["name"] == name)
            .unwrap_or_else(|| panic!("missing entry {name}"))
    };
    assert_eq!(
        by_name("src"),
        &json!({ "name": "src", "type": "tree", "mode": "040000", "size": null, "target": null })
    );
    assert_eq!(
        by_name("README.md"),
        &json!({ "name": "README.md", "type": "blob", "mode": "100644", "size": 16, "target": null })
    );
    // A symlink carries its target; `size` stays blob-only (docs/API.md).
    assert_eq!(
        by_name("link"),
        &json!({ "name": "link", "type": "symlink", "mode": "120000", "size": null, "target": "README.md" })
    );
}

#[tokio::test]
async fn tree_should_list_subdirectories_and_gitlinks() {
    let (root, _, _) = setup();
    let body = get_ok(root.path(), "/api/v1/repos/files/tree/main/src").await;
    assert_eq!(body["path"], "src");
    let entries = body["entries"].as_array().unwrap();
    let names: Vec<&str> = entries
        .iter()
        .map(|entry| entry["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["lib", "main.rs", "readme-link"]);
    // The target is stored verbatim, relative to the entry's own directory —
    // never resolved server-side, so `..` reaches the caller intact.
    assert_eq!(entries[2]["target"], "../README.md");
    assert_eq!(entries[1]["target"], Value::Null, "a blob has no target");

    let body = get_ok(root.path(), "/api/v1/repos/files/tree/main/vendor").await;
    assert_eq!(
        body["entries"],
        json!([{ "name": "dep", "type": "commit", "mode": "160000", "size": null, "target": null }])
    );
}

#[tokio::test]
async fn tree_should_return_404_for_missing_or_non_directory_paths() {
    let (root, _, _) = setup();
    for uri in [
        "/api/v1/repos/files/tree/main/nope",
        "/api/v1/repos/files/tree/main/README.md",
    ] {
        assert_error(root.path(), uri, StatusCode::NOT_FOUND, "path_not_found").await;
    }
    assert_error(
        root.path(),
        "/api/v1/repos/files/tree/no-such-branch",
        StatusCode::NOT_FOUND,
        "ref_not_found",
    )
    .await;
}

#[tokio::test]
async fn tree_should_reject_dot_segments() {
    let (root, _, _) = setup();
    assert_error(
        root.path(),
        "/api/v1/repos/files/tree/main/../../etc",
        StatusCode::BAD_REQUEST,
        "invalid_param",
    )
    .await;
}

// --- ref/path splitting ---

#[tokio::test]
async fn slash_branch_should_win_longest_ref_match() {
    let (root, _, branch) = setup();
    // `feature/x` is both a branch and a directory on main; the branch wins.
    let body = get_ok(root.path(), "/api/v1/repos/files/tree/feature/x").await;
    assert_eq!(body["sha"], branch);
    assert_eq!(body["path"], "");
    let names: Vec<&str> = body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"branch-file.txt"), "got {names:?}");

    // The directory is still reachable through an explicit ref prefix.
    let body = get_ok(root.path(), "/api/v1/repos/files/tree/feature/x/feature/x").await;
    assert_eq!(body["path"], "feature/x");
    assert_eq!(body["entries"][0]["name"], "inside.txt");
    let body = get_ok(
        root.path(),
        "/api/v1/repos/files/blob/main/feature/x/inside.txt",
    )
    .await;
    assert_eq!(body["content"], "from main\n");
}

#[tokio::test]
async fn sha_addressed_responses_should_be_immutable() {
    let (root, head, _) = setup();
    let (status, headers, body) = common::get_json_with_headers(
        router_for(root.path()),
        &format!("/api/v1/repos/files/tree/{head}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {body}");
    assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), IMMUTABLE);

    // Branch names and abbreviated shas are mutable addresses: ETag + no-cache.
    for uri in [
        "/api/v1/repos/files/tree/main".to_owned(),
        format!("/api/v1/repos/files/blob/{}/README.md", &head[..10]),
    ] {
        let (status, headers, body) =
            common::get_json_with_headers(router_for(root.path()), &uri).await;
        assert_eq!(status, StatusCode::OK, "unexpected response: {body}");
        assert_eq!(
            headers.get(header::CACHE_CONTROL).unwrap(),
            "no-cache",
            "uri {uri}"
        );
        assert!(headers.contains_key(header::ETAG), "uri {uri}");
    }
}

// --- blob ---

#[tokio::test]
async fn blob_should_return_text_content() {
    let (root, head, _) = setup();
    let body = get_ok(root.path(), "/api/v1/repos/files/blob/main/README.md").await;
    assert_eq!(
        body,
        json!({
            "sha": head,
            "path": "README.md",
            "mode": "100644",
            "size": 16,
            "binary": false,
            "too_large": false,
            "content": "# Files fixture\n"
        })
    );
}

#[tokio::test]
async fn blob_should_flag_binary_and_oversized_files() {
    let (root, _, _) = setup();
    let body = get_ok(root.path(), "/api/v1/repos/files/blob/main/assets/logo.png").await;
    assert_eq!(body["binary"], true);
    assert_eq!(body["content"], Value::Null);

    let body = get_ok(root.path(), "/api/v1/repos/files/blob/main/big.txt").await;
    assert_eq!(body["too_large"], true);
    assert_eq!(body["binary"], false);
    assert_eq!(body["size"], 1024 * 1024 + 1);
    assert_eq!(body["content"], Value::Null);
}

#[tokio::test]
async fn blob_should_expose_symlinks_as_target_paths() {
    let (root, _, _) = setup();
    let body = get_ok(root.path(), "/api/v1/repos/files/blob/main/link").await;
    assert_eq!(body["mode"], "120000");
    assert_eq!(body["content"], "README.md");
}

#[tokio::test]
async fn blob_should_return_404_for_non_files() {
    let (root, _, _) = setup();
    for uri in [
        "/api/v1/repos/files/blob/main/src",
        "/api/v1/repos/files/blob/main/vendor/dep",
        "/api/v1/repos/files/blob/main",
    ] {
        assert_error(root.path(), uri, StatusCode::NOT_FOUND, "path_not_found").await;
    }
}

// --- raw ---

#[tokio::test]
async fn raw_should_stream_bytes_with_detected_mime() {
    let (root, head, _) = setup();
    let cases = [
        ("README.md", "text/markdown", b"# Files fixture\n".to_vec()),
        (
            "assets/logo.png",
            "image/png",
            b"\x89PNG\r\n\x1a\n\x00binary".to_vec(),
        ),
        (
            "noext",
            "text/plain; charset=utf-8",
            b"plain text\n".to_vec(),
        ),
    ];
    for (path, mime, bytes) in cases {
        let (status, headers, body) = common::get_bytes_with_headers(
            router_for(root.path()),
            &format!("/api/v1/repos/files/raw/main/{path}"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "path {path}");
        assert_eq!(headers.get(header::CONTENT_TYPE).unwrap(), mime);
        assert_eq!(
            headers.get(header::X_CONTENT_TYPE_OPTIONS).unwrap(),
            "nosniff"
        );
        // Branch-addressed raw is mutable: ETag + no-cache.
        assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "no-cache");
        assert!(headers.contains_key(header::ETAG), "path {path}");
        assert_eq!(body, bytes, "path {path}");
    }

    let (status, headers, _) = common::get_bytes_with_headers(
        router_for(root.path()),
        &format!("/api/v1/repos/files/raw/{head}/README.md"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), IMMUTABLE);
}

// --- readme ---

#[tokio::test]
async fn readme_should_find_markdown_on_head_and_refs() {
    let (root, head, _) = setup();
    let body = get_ok(root.path(), "/api/v1/repos/files/readme").await;
    assert_eq!(
        body,
        json!({ "path": "README.md", "format": "markdown", "content": "# Files fixture\n" })
    );
    let body = get_ok(root.path(), "/api/v1/repos/files/readme?ref=feature/x").await;
    assert_eq!(body["path"], "README.md");

    let (status, headers, body) = common::get_json_with_headers(
        router_for(root.path()),
        &format!("/api/v1/repos/files/readme?ref={head}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {body}");
    assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), IMMUTABLE);

    assert_error(
        root.path(),
        "/api/v1/repos/files/readme?ref=no-such-ref",
        StatusCode::NOT_FOUND,
        "ref_not_found",
    )
    .await;
}

#[tokio::test]
async fn readme_should_match_case_insensitively_in_priority_order() {
    let root = tempfile::tempdir().unwrap();
    let bare = common::create_bare_repo(root.path(), "lower.git");
    let work = tempfile::tempdir().unwrap();
    common::git(
        work.path(),
        &["clone", "--quiet", bare.to_str().unwrap(), "."],
    );
    std::fs::write(work.path().join("readme.md"), "lower readme\n").unwrap();
    std::fs::write(work.path().join("README.txt"), "txt readme\n").unwrap();
    common::commit_all(work.path(), "feat: readmes");
    common::git(work.path(), &["push", "--quiet", "origin", "HEAD:main"]);

    // `.md` beats `.txt` even though only the latter matches exactly.
    let body = get_ok(root.path(), "/api/v1/repos/lower/readme").await;
    assert_eq!(
        body,
        json!({ "path": "readme.md", "format": "markdown", "content": "lower readme\n" })
    );
}

#[tokio::test]
async fn readme_should_fall_back_to_plain_and_404_when_absent() {
    let root = tempfile::tempdir().unwrap();
    let plain = common::create_bare_repo(root.path(), "plain.git");
    common::add_commit(&plain, "README", "plain readme\n", "feat: readme");
    let body = get_ok(root.path(), "/api/v1/repos/plain/readme").await;
    assert_eq!(
        body,
        json!({ "path": "README", "format": "plain", "content": "plain readme\n" })
    );

    let none = common::create_bare_repo(root.path(), "noreadme.git");
    common::add_commit(&none, "file.txt", "hi\n", "feat: file");
    assert_error(
        root.path(),
        "/api/v1/repos/noreadme/readme",
        StatusCode::NOT_FOUND,
        "path_not_found",
    )
    .await;
}

// --- empty repository ---

#[tokio::test]
async fn empty_repo_should_return_ref_not_found() {
    let root = tempfile::tempdir().unwrap();
    common::create_bare_repo(root.path(), "empty.git");
    for uri in [
        "/api/v1/repos/empty/tree/main",
        "/api/v1/repos/empty/blob/main/a.txt",
        "/api/v1/repos/empty/raw/main/a.txt",
        "/api/v1/repos/empty/readme",
    ] {
        assert_error(root.path(), uri, StatusCode::NOT_FOUND, "ref_not_found").await;
    }
}
