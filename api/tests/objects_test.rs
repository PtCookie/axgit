//! Integration tests for `GET /api/v1/repos/{repo}/objects/{oid}` and
//! `.../objects/{oid}/raw` — the one by-oid entry point in this API.

mod common;

use std::path::Path;

use axum::http::{StatusCode, header};
use serde_json::Value;

use common::router_for;

const IMMUTABLE: &str = "public, max-age=31536000, immutable";

async fn get_ok(repo_root: &Path, uri: &str) -> Value {
    let (status, json) = common::get_json(router_for(repo_root), uri).await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");
    json
}

/// `alpha.git`: one commit on `main` with a root blob (`a.txt`) and a
/// subdirectory (`dir/nested.txt`) — enough to exercise a tree with more
/// than one entry, a nested tree, and two distinct blobs. Also a binary
/// blob (`bin.dat`, contains a NUL byte) and an annotated tag on the root
/// tree (`tree-tag`, not pushed to `refs/heads/*` — reached only by oid).
struct Fixture {
    root: tempfile::TempDir,
    commit_sha: String,
    root_tree_sha: String,
    a_txt_sha: String,
    dir_tree_sha: String,
    nested_txt_sha: String,
    bin_dat_sha: String,
    /// The annotated `tree-tag` tag object's own oid (not peeled).
    tag_object_sha: String,
}

fn setup() -> Fixture {
    let root = tempfile::tempdir().expect("failed to create fixture root");
    let bare = common::create_bare_repo(root.path(), "alpha.git");

    let work = tempfile::tempdir().expect("failed to create work dir");
    let work_path = work.path();
    common::git(
        work_path,
        &["init", "--quiet", "--initial-branch=main", "."],
    );
    std::fs::write(work_path.join("a.txt"), "hello\n").unwrap();
    std::fs::create_dir(work_path.join("dir")).unwrap();
    std::fs::write(work_path.join("dir/nested.txt"), "nested\n").unwrap();
    // A NUL byte trips libgit2's binary heuristic — every byte here is
    // still ASCII, so this stays a valid `&str` literal.
    std::fs::write(work_path.join("bin.dat"), "\0\x01\x02binary").unwrap();
    common::git(work_path, &["add", "."]);
    common::git(work_path, &["commit", "--quiet", "-m", "initial"]);
    common::git(
        work_path,
        &["push", "--quiet", bare.to_str().unwrap(), "main:main"],
    );

    let commit_sha = common::git_output(&bare, &["rev-parse", "main"], &[]);
    let root_tree_sha = common::git_output(&bare, &["rev-parse", "main^{tree}"], &[]);
    let a_txt_sha = common::git_output(&bare, &["rev-parse", "main:a.txt"], &[]);
    let dir_tree_sha = common::git_output(&bare, &["rev-parse", "main:dir"], &[]);
    let nested_txt_sha = common::git_output(&bare, &["rev-parse", "main:dir/nested.txt"], &[]);
    let bin_dat_sha = common::git_output(&bare, &["rev-parse", "main:bin.dat"], &[]);

    common::add_annotated_tag_on(&bare, "tree-tag", "a tree tag", &root_tree_sha);
    // Unlike `main:a.txt`, `rev-parse` of an annotated tag *name* returns the
    // tag object's own (unpeeled) oid — the object this test wants to fetch
    // by id, not its target.
    let tag_object_sha = common::git_output(&bare, &["rev-parse", "tree-tag"], &[]);

    Fixture {
        root,
        commit_sha,
        root_tree_sha,
        a_txt_sha,
        dir_tree_sha,
        nested_txt_sha,
        bin_dat_sha,
        tag_object_sha,
    }
}

#[tokio::test]
async fn a_tree_oid_should_list_its_entries_with_their_own_sha() {
    let fixture = setup();
    let json = get_ok(
        fixture.root.path(),
        &format!("/api/v1/repos/alpha/objects/{}", fixture.root_tree_sha),
    )
    .await;

    assert_eq!(json["sha"], fixture.root_tree_sha);
    assert_eq!(json["type"], "tree");
    assert_eq!(json["blob"], Value::Null);
    assert_eq!(json["tag"], Value::Null);

    let entries = json["tree"]["entries"].as_array().expect("entries missing");
    // Trees first, then by name ascending — same ordering as `GET /tree`.
    let names: Vec<&str> = entries
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["dir", "a.txt", "bin.dat"],
        "unexpected order: {json}"
    );

    let dir_entry = &entries[0];
    assert_eq!(dir_entry["type"], "tree");
    assert_eq!(dir_entry["sha"], fixture.dir_tree_sha);
    let a_txt_entry = &entries[1];
    assert_eq!(a_txt_entry["type"], "blob");
    assert_eq!(a_txt_entry["sha"], fixture.a_txt_sha);
}

#[tokio::test]
async fn a_nested_tree_oid_should_list_its_own_entries() {
    let fixture = setup();
    let json = get_ok(
        fixture.root.path(),
        &format!("/api/v1/repos/alpha/objects/{}", fixture.dir_tree_sha),
    )
    .await;

    assert_eq!(json["type"], "tree");
    let entries = json["tree"]["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["name"], "nested.txt");
    assert_eq!(entries[0]["sha"], fixture.nested_txt_sha);
}

#[tokio::test]
async fn a_blob_oid_should_report_its_content() {
    let fixture = setup();
    let json = get_ok(
        fixture.root.path(),
        &format!("/api/v1/repos/alpha/objects/{}", fixture.a_txt_sha),
    )
    .await;

    assert_eq!(json["sha"], fixture.a_txt_sha);
    assert_eq!(json["type"], "blob");
    assert_eq!(json["tree"], Value::Null);
    assert_eq!(json["tag"], Value::Null);
    assert_eq!(json["blob"]["binary"], false);
    assert_eq!(json["blob"]["too_large"], false);
    assert_eq!(json["blob"]["content"], "hello\n");
    assert_eq!(json["blob"]["size"], 6);
}

#[tokio::test]
async fn a_binary_blob_oid_should_null_content_and_flag_binary() {
    let fixture = setup();
    let json = get_ok(
        fixture.root.path(),
        &format!("/api/v1/repos/alpha/objects/{}", fixture.bin_dat_sha),
    )
    .await;

    assert_eq!(json["blob"]["binary"], true);
    assert_eq!(json["blob"]["content"], Value::Null);
}

#[tokio::test]
async fn a_commit_oid_should_null_every_payload() {
    let fixture = setup();
    let json = get_ok(
        fixture.root.path(),
        &format!("/api/v1/repos/alpha/objects/{}", fixture.commit_sha),
    )
    .await;

    assert_eq!(json["sha"], fixture.commit_sha);
    assert_eq!(json["type"], "commit");
    assert_eq!(
        (
            json["tree"].clone(),
            json["blob"].clone(),
            json["tag"].clone()
        ),
        (Value::Null, Value::Null, Value::Null),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn a_tag_oid_should_report_its_dereference_and_message() {
    let fixture = setup();
    let json = get_ok(
        fixture.root.path(),
        &format!("/api/v1/repos/alpha/objects/{}", fixture.tag_object_sha),
    )
    .await;

    assert_eq!(json["sha"], fixture.tag_object_sha);
    assert_eq!(json["type"], "tag");
    assert_eq!(json["tree"], Value::Null);
    assert_eq!(json["blob"], Value::Null);
    assert_eq!(json["tag"]["object"]["sha"], fixture.root_tree_sha);
    assert_eq!(json["tag"]["object"]["type"], "tree");
    assert_eq!(
        json["tag"]["target"],
        Value::Null,
        "a tag on a tree never reaches a commit"
    );
    assert_eq!(json["tag"]["message"], "a tree tag");
}

#[tokio::test]
async fn an_unknown_oid_should_be_object_not_found() {
    let fixture = setup();
    let unknown = "0".repeat(40);
    let (status, json) = common::get_json(
        router_for(fixture.root.path()),
        &format!("/api/v1/repos/alpha/objects/{unknown}"),
    )
    .await;

    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("object_not_found")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn an_abbreviated_oid_should_be_invalid_param() {
    let fixture = setup();
    let short = &fixture.a_txt_sha[..10];
    let (status, json) = common::get_json(
        router_for(fixture.root.path()),
        &format!("/api/v1/repos/alpha/objects/{short}"),
    )
    .await;

    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("invalid_param")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn a_non_hex_oid_should_be_invalid_param() {
    let fixture = setup();
    let bogus = "z".repeat(40);
    let (status, json) = common::get_json(
        router_for(fixture.root.path()),
        &format!("/api/v1/repos/alpha/objects/{bogus}"),
    )
    .await;

    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("invalid_param")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn unknown_repo_should_be_repo_not_found() {
    let root = tempfile::tempdir().unwrap();
    let sha = "0".repeat(40);
    let (status, json) = common::get_json(
        router_for(root.path()),
        &format!("/api/v1/repos/does-not-exist/objects/{sha}"),
    )
    .await;

    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("repo_not_found")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn the_object_endpoint_should_always_be_immutable() {
    let fixture = setup();
    let (status, headers, body) = common::get_bytes_with_headers(
        router_for(fixture.root.path()),
        &format!("/api/v1/repos/alpha/objects/{}", fixture.a_txt_sha),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CACHE_CONTROL], IMMUTABLE);
    assert!(
        !headers.contains_key(header::ETAG),
        "immutable responses carry no ETag"
    );
    assert!(!body.is_empty());
}

#[tokio::test]
async fn raw_should_stream_the_blob_bytes_and_be_immutable() {
    let fixture = setup();
    let (status, headers, body) = common::get_bytes_with_headers(
        router_for(fixture.root.path()),
        &format!("/api/v1/repos/alpha/objects/{}/raw", fixture.a_txt_sha),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CACHE_CONTROL], IMMUTABLE);
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(headers[header::CONTENT_TYPE], "text/plain; charset=utf-8");
    assert_eq!(body, b"hello\n");
}

#[tokio::test]
async fn raw_on_a_non_blob_oid_should_be_object_not_found() {
    let fixture = setup();
    let (status, json) = common::get_json(
        router_for(fixture.root.path()),
        &format!("/api/v1/repos/alpha/objects/{}/raw", fixture.commit_sha),
    )
    .await;

    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("object_not_found")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn raw_on_an_abbreviated_oid_should_be_invalid_param() {
    let fixture = setup();
    let short = &fixture.a_txt_sha[..10];
    let (status, json) = common::get_json(
        router_for(fixture.root.path()),
        &format!("/api/v1/repos/alpha/objects/{short}/raw"),
    )
    .await;

    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("invalid_param")),
        "unexpected response: {json}"
    );
}
