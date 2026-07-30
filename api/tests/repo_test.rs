//! Integration tests for `GET /api/v1/repos/{repo}` and `.../refs`.

mod common;

use std::path::Path;

use axum::Router;
use axum::http::StatusCode;
use serde_json::Value;
use tempfile::TempDir;

use axgit::routes::build_router;
use axgit::state::AppState;

fn router_for(repo_root: &Path, clone_url_base: Option<&str>) -> Router {
    let mut config = common::test_config(repo_root);
    config.clone_url_base = clone_url_base.map(str::to_owned);
    build_router(AppState::new(config))
}

/// Fixture set: `alpha.git` (one commit on `main`, branch `dev`,
/// annotated tag `v1.0.0`, lightweight tag `snapshot`), `empty.git` (no commits).
fn setup_fixtures() -> TempDir {
    let root = tempfile::tempdir().expect("failed to create fixture root");

    let alpha = common::create_bare_repo(root.path(), "alpha.git");
    common::set_meta(&alpha, "cgit", "desc", "Alpha repository");
    common::add_commit(&alpha, "README.md", "# alpha\n", "feat: initial commit");
    common::add_branch(&alpha, "dev");
    common::add_annotated_tag(&alpha, "v1.0.0", "release v1.0.0");
    common::add_lightweight_tag(&alpha, "snapshot");

    common::create_bare_repo(root.path(), "empty.git");

    root
}

async fn get_ok(root: &Path, uri: &str) -> Value {
    let (status, json) = common::get_json(router_for(root, None), uri).await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");
    json
}

#[tokio::test]
async fn get_repo_returns_summary_with_head_and_counts() {
    let root = setup_fixtures();

    let json = get_ok(root.path(), "/api/v1/repos/alpha").await;

    let head = json["head"].as_str().expect("head is not a string");
    assert_eq!(head.len(), 40, "head is not a full sha: {json}");
    assert_eq!(
        (
            json["description"].as_str(),
            json["default_branch"].as_str(),
            json["branch_count"].as_u64(),
            json["tag_count"].as_u64(),
        ),
        (Some("Alpha repository"), Some("main"), Some(2), Some(2)),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn get_repo_builds_clone_url_from_configured_base() {
    let root = setup_fixtures();

    let (status, json) = common::get_json(
        router_for(root.path(), Some("http://git.example.com/")),
        "/api/v1/repos/alpha",
    )
    .await;

    assert_eq!(
        (status, json["clone_url"].as_str()),
        (StatusCode::OK, Some("http://git.example.com/alpha.git")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn get_repo_returns_null_clone_url_without_base() {
    let root = setup_fixtures();

    let json = get_ok(root.path(), "/api/v1/repos/alpha").await;

    assert_eq!(json["clone_url"], Value::Null);
}

#[tokio::test]
async fn get_repo_returns_404_for_missing_repo() {
    let root = setup_fixtures();

    let (status, json) = common::get_json(
        router_for(root.path(), None),
        "/api/v1/repos/does-not-exist",
    )
    .await;

    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("repo_not_found")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn get_repo_rejects_path_traversal_names() {
    let root = setup_fixtures();

    for uri in ["/api/v1/repos/..%2f..%2fetc", "/api/v1/repos/.hidden"] {
        let (status, json) = common::get_json(router_for(root.path(), None), uri).await;
        assert_eq!(
            (status, json["error"]["code"].as_str()),
            (StatusCode::BAD_REQUEST, Some("invalid_param")),
            "unexpected response for {uri}: {json}"
        );
    }
}

#[tokio::test]
async fn get_repo_returns_nulls_and_zero_counts_for_empty_repo() {
    let root = setup_fixtures();

    let json = get_ok(root.path(), "/api/v1/repos/empty").await;

    assert_eq!(
        (
            json["head"].clone(),
            json["default_branch"].clone(),
            json["branch_count"].as_u64(),
            json["tag_count"].as_u64(),
        ),
        (Value::Null, Value::Null, Some(0), Some(0)),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn get_refs_lists_branches_sorted_by_name() {
    let root = setup_fixtures();

    let json = get_ok(root.path(), "/api/v1/repos/alpha/refs").await;

    let branches = json["branches"]
        .as_array()
        .expect("branches is not an array");
    let names: Vec<&str> = branches
        .iter()
        .map(|branch| branch["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["dev", "main"], "unexpected response: {json}");
    // Both branches point at the same single commit.
    assert_eq!(branches[0]["target"], branches[1]["target"]);
    assert_eq!(branches[1]["committed_at"], common::FIXED_DATE);
}

#[tokio::test]
async fn get_refs_returns_annotated_tag_with_annotation_and_tagged_at() {
    let root = setup_fixtures();

    let json = get_ok(root.path(), "/api/v1/repos/alpha/refs").await;

    let tag = &json["tags"][1];
    assert_eq!(
        (
            tag["name"].as_str(),
            tag["annotation"].as_str(),
            tag["tagged_at"].as_str(),
        ),
        (
            Some("v1.0.0"),
            Some("release v1.0.0"),
            Some(common::FIXED_DATE)
        ),
        "unexpected response: {json}"
    );
    // Annotated tag target is the peeled commit, not the tag object.
    assert_eq!(tag["target"], json["branches"][1]["target"]);
}

#[tokio::test]
async fn get_refs_returns_lightweight_tag_with_null_annotation() {
    let root = setup_fixtures();

    let json = get_ok(root.path(), "/api/v1/repos/alpha/refs").await;

    let tag = &json["tags"][0];
    assert_eq!(
        (
            tag["name"].as_str(),
            tag["annotation"].clone(),
            tag["tagged_at"].clone(),
        ),
        (Some("snapshot"), Value::Null, Value::Null),
        "unexpected response: {json}"
    );
    assert_eq!(tag["target"], json["branches"][1]["target"]);
}

#[tokio::test]
async fn get_refs_returns_empty_arrays_for_empty_repo() {
    let root = setup_fixtures();

    let json = get_ok(root.path(), "/api/v1/repos/empty/refs").await;

    assert_eq!(
        (
            json["branches"].as_array().map(Vec::len),
            json["tags"].as_array().map(Vec::len),
        ),
        (Some(0), Some(0)),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn get_refs_returns_404_for_missing_repo() {
    let root = setup_fixtures();

    let (status, json) = common::get_json(
        router_for(root.path(), None),
        "/api/v1/repos/does-not-exist/refs",
    )
    .await;

    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("repo_not_found")),
        "unexpected response: {json}"
    );
}
