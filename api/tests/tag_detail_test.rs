//! Integration tests for `GET /api/v1/repos/{repo}/tags/{name}`.

mod common;

use std::path::Path;

use axum::http::{StatusCode, header};
use serde_json::Value;

use common::router_for;

/// sha256 of `committer@example.com` (the fixed fixture committer email —
/// `git tag -a` takes the tagger identity from the committer, not the author).
const TAGGER_EMAIL_HASH: &str = "f395b38df60e606322b6576159c903509f1a217b386580d8150965d33d8ef30f";

async fn get_ok(repo_root: &Path, uri: &str) -> Value {
    let (status, json) = common::get_json(router_for(repo_root), uri).await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");
    json
}

/// `alpha.git`: one commit on `main`, plus:
/// - `v1.0.0` — annotated, multi-line message, on `main`
/// - `snapshot` — lightweight, on `main`
/// - `release/1.0` — lightweight, name contains `/`, on `main`
/// - `blob-tag` — annotated, on `a.txt`'s blob (not a commit)
/// - `tree-tag` — annotated, on `main`'s root tree (not a commit)
/// - `inner` / `outer` — annotated tag on `main`, then annotated tag on `inner`
fn setup() -> (tempfile::TempDir, String) {
    let root = tempfile::tempdir().expect("failed to create fixture root");
    let bare = common::create_bare_repo(root.path(), "alpha.git");
    common::add_commit(&bare, "a.txt", "one\n", "initial");
    let head = common::git_output(&bare, &["rev-parse", "main"], &[]);

    common::add_annotated_tag(&bare, "v1.0.0", "Release v1.0.0\n\nSecond line.");
    common::add_lightweight_tag(&bare, "snapshot");
    common::add_lightweight_tag(&bare, "release/1.0");

    let blob = common::git_output(&bare, &["rev-parse", "main:a.txt"], &[]);
    common::add_annotated_tag_on(&bare, "blob-tag", "a blob tag", &blob);
    let tree = common::git_output(&bare, &["rev-parse", "main^{tree}"], &[]);
    common::add_annotated_tag_on(&bare, "tree-tag", "a tree tag", &tree);

    common::add_annotated_tag_on(&bare, "inner", "inner tag", "main");
    let inner = common::git_output(&bare, &["rev-parse", "inner"], &[]);
    common::add_annotated_tag_on(&bare, "outer", "outer tag", &inner);

    (root, head)
}

#[tokio::test]
async fn annotated_tag_should_return_full_message_tagger_and_commit_object() {
    let (root, head) = setup();
    let json = get_ok(root.path(), "/api/v1/repos/alpha/tags/v1.0.0").await;

    assert_eq!(json["name"], "v1.0.0");
    assert_eq!(json["target"], head);
    assert_eq!(json["object"]["sha"], head);
    assert_eq!(json["object"]["type"], "commit");
    // The tag object's own sha is neither null nor the commit it points at.
    let tag_object = json["tag_object"].as_str().expect("tag_object missing");
    assert_ne!(tag_object, head);
    assert_eq!(json["message"], "Release v1.0.0\n\nSecond line.");
    assert_eq!(json["tagger"]["name"], "Test Committer");
    assert_eq!(json["tagger"]["email_hash"], TAGGER_EMAIL_HASH);
    assert_eq!(json["tagged_at"], common::FIXED_DATE);
}

#[tokio::test]
async fn lightweight_tag_should_null_tag_object_message_and_tagger() {
    let (root, head) = setup();
    let json = get_ok(root.path(), "/api/v1/repos/alpha/tags/snapshot").await;

    assert_eq!(
        (
            json["tag_object"].clone(),
            json["message"].clone(),
            json["tagger"].clone(),
            json["tagged_at"].clone(),
        ),
        (Value::Null, Value::Null, Value::Null, Value::Null),
        "unexpected response: {json}"
    );
    assert_eq!(json["object"]["sha"], head);
    assert_eq!(json["object"]["type"], "commit");
    assert_eq!(json["target"], head);
}

#[tokio::test]
async fn tag_on_a_blob_should_report_blob_object_and_null_target() {
    let (root, _head) = setup();
    let json = get_ok(root.path(), "/api/v1/repos/alpha/tags/blob-tag").await;

    assert_eq!(json["object"]["type"], "blob");
    assert_eq!(
        json["target"],
        Value::Null,
        "a blob tag never reaches a commit"
    );
    assert_eq!(json["message"], "a blob tag");
}

#[tokio::test]
async fn tag_on_a_tree_should_report_tree_object_and_null_target() {
    let (root, _head) = setup();
    let json = get_ok(root.path(), "/api/v1/repos/alpha/tags/tree-tag").await;

    assert_eq!(json["object"]["type"], "tree");
    assert_eq!(json["target"], Value::Null);
}

#[tokio::test]
async fn nested_tag_should_report_the_inner_tag_as_the_object() {
    let (root, head) = setup();
    let inner_json = get_ok(root.path(), "/api/v1/repos/alpha/tags/inner").await;
    let inner_tag_object = inner_json["tag_object"].as_str().unwrap().to_owned();

    let json = get_ok(root.path(), "/api/v1/repos/alpha/tags/outer").await;
    assert_eq!(json["object"]["type"], "tag");
    assert_eq!(json["object"]["sha"], inner_tag_object);
    assert_eq!(
        json["target"], head,
        "target fully peels through both tags to the commit"
    );
}

#[tokio::test]
async fn tag_name_with_a_slash_should_resolve() {
    let (root, head) = setup();
    let json = get_ok(root.path(), "/api/v1/repos/alpha/tags/release/1.0").await;

    assert_eq!(json["name"], "release/1.0");
    assert_eq!(json["target"], head);
}

#[tokio::test]
async fn trailing_slash_should_resolve() {
    let (root, _head) = setup();
    let json = get_ok(root.path(), "/api/v1/repos/alpha/tags/v1.0.0/").await;
    assert_eq!(json["name"], "v1.0.0");
}

#[tokio::test]
async fn unknown_tag_should_be_ref_not_found() {
    let (root, _head) = setup();
    let (status, json) = common::get_json(
        router_for(root.path()),
        "/api/v1/repos/alpha/tags/no-such-tag",
    )
    .await;

    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("ref_not_found")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn branch_sha_and_head_should_not_resolve_as_tags() {
    let (root, head) = setup();
    for candidate in ["main", head.as_str(), "HEAD"] {
        let uri = format!("/api/v1/repos/alpha/tags/{candidate}");
        let (status, json) = common::get_json(router_for(root.path()), &uri).await;
        assert_eq!(
            (status, json["error"]["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("ref_not_found")),
            "{candidate} unexpectedly resolved: {json}"
        );
    }
}

#[tokio::test]
async fn missing_tag_name_should_be_not_found() {
    let (root, _head) = setup();
    for uri in ["/api/v1/repos/alpha/tags", "/api/v1/repos/alpha/tags/"] {
        let (status, json) = common::get_json(router_for(root.path()), uri).await;
        assert_eq!(
            (status, json["error"]["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("not_found")),
            "unexpected response for {uri}: {json}"
        );
    }
}

#[tokio::test]
async fn unknown_repo_should_be_repo_not_found() {
    let root = tempfile::tempdir().unwrap();
    let (status, json) = common::get_json(
        router_for(root.path()),
        "/api/v1/repos/does-not-exist/tags/v1.0.0",
    )
    .await;

    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("repo_not_found")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn tag_detail_should_use_etag_and_never_be_immutable() {
    let (root, _head) = setup();
    let router = router_for(root.path());

    let (status, headers, json) =
        common::get_json_with_headers(router.clone(), "/api/v1/repos/alpha/tags/v1.0.0").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
    let etag = headers[header::ETAG].to_str().unwrap().to_owned();
    assert!(!json.to_string().contains("immutable"));

    let (status, headers, body) = common::get_bytes_with_request_headers(
        router,
        "/api/v1/repos/alpha/tags/v1.0.0",
        &[("if-none-match", &etag)],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert!(body.is_empty());
    assert_eq!(headers[header::ETAG].to_str().unwrap(), etag);
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
}

#[tokio::test]
async fn response_should_not_expose_a_raw_email() {
    let (root, _head) = setup();
    let json = get_ok(root.path(), "/api/v1/repos/alpha/tags/v1.0.0").await;
    assert!(
        !json.to_string().contains('@'),
        "response leaked a raw email address: {json}"
    );
}
