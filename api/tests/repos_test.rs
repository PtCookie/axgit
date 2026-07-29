mod common;

use std::net::SocketAddr;
use std::path::Path;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tempfile::TempDir;
use tower::ServiceExt;

use axgit::config::Config;
use axgit::routes::build_router;
use axgit::state::AppState;

fn router_for(repo_root: &Path) -> Router {
    let config = Config {
        repo_root: repo_root.to_owned(),
        static_dir: None,
        listen: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
        clone_url_base: None,
        cache_scan_ttl_secs: 60,
    };
    build_router(AppState::new(config))
}

/// Fixture set: `alpha.git` ([cgit] meta + agefile + commit),
/// `bravo.git` ([axgit] overriding [cgit], no agefile, commit),
/// `empty.git` (no commits, no meta).
fn setup_fixtures() -> TempDir {
    let root = tempfile::tempdir().expect("failed to create fixture root");

    let alpha = common::create_bare_repo(root.path(), "alpha.git");
    common::set_meta(&alpha, "cgit", "section", "infra");
    common::set_meta(&alpha, "cgit", "owner", "PtCookie");
    common::set_meta(&alpha, "cgit", "desc", "Alpha repository");
    common::add_commit(&alpha, "README.md", "# alpha\n", "feat: initial commit");
    common::write_agefile(&alpha, "2026-07-24 13:06:00 +0900\n");

    let bravo = common::create_bare_repo(root.path(), "bravo.git");
    common::set_meta(&bravo, "cgit", "desc", "cgit description");
    common::set_meta(&bravo, "axgit", "desc", "axgit description");
    common::add_commit(&bravo, "README.md", "# bravo\n", "feat: initial commit");

    common::create_bare_repo(root.path(), "empty.git");

    root
}

async fn list_repos(root: &Path) -> Value {
    let (status, json) = common::get_json(router_for(root), "/api/v1/repos").await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");
    json
}

#[tokio::test]
async fn list_repos_returns_repos_sorted_by_name() {
    let root = setup_fixtures();

    let json = list_repos(root.path()).await;

    let names: Vec<&str> = json["repos"]
        .as_array()
        .expect("repos is not an array")
        .iter()
        .map(|repo| repo["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["alpha", "bravo", "empty"]);
}

#[tokio::test]
async fn list_repos_reads_cgit_metadata() {
    let root = setup_fixtures();

    let json = list_repos(root.path()).await;

    let alpha = &json["repos"][0];
    assert_eq!(
        (
            alpha["section"].as_str(),
            alpha["owner"].as_str(),
            alpha["description"].as_str(),
            alpha["default_branch"].as_str(),
        ),
        (
            Some("infra"),
            Some("PtCookie"),
            Some("Alpha repository"),
            Some("main")
        ),
    );
}

#[tokio::test]
async fn list_repos_prefers_axgit_section_over_cgit() {
    let root = setup_fixtures();

    let json = list_repos(root.path()).await;

    assert_eq!(json["repos"][1]["description"], "axgit description");
}

#[tokio::test]
async fn list_repos_reads_last_modified_from_agefile() {
    let root = setup_fixtures();

    let json = list_repos(root.path()).await;

    assert_eq!(
        json["repos"][0]["last_modified"],
        "2026-07-24T13:06:00+09:00"
    );
}

#[tokio::test]
async fn list_repos_falls_back_to_head_authordate_without_agefile() {
    let root = setup_fixtures();

    let json = list_repos(root.path()).await;

    assert_eq!(json["repos"][1]["last_modified"], common::FIXED_DATE);
}

#[tokio::test]
async fn list_repos_returns_null_fields_for_empty_repo() {
    let root = setup_fixtures();

    let json = list_repos(root.path()).await;

    let empty = &json["repos"][2];
    assert_eq!(
        (
            empty["default_branch"].clone(),
            empty["last_modified"].clone()
        ),
        (Value::Null, Value::Null),
    );
}

#[tokio::test]
async fn unimplemented_endpoint_returns_501() {
    let root = setup_fixtures();

    let (status, json) =
        common::get_json(router_for(root.path()), "/api/v1/repos/alpha/tree/main").await;

    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::NOT_IMPLEMENTED, Some("not_implemented")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn receive_pack_returns_403_read_only() {
    let root = setup_fixtures();

    let response = router_for(root.path())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/alpha.git/git-receive-pack")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("read_only")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn info_refs_with_receive_pack_service_returns_403() {
    let root = setup_fixtures();

    let (status, json) = common::get_json(
        router_for(root.path()),
        "/alpha.git/info/refs?service=git-receive-pack",
    )
    .await;

    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("read_only")),
        "unexpected response: {json}"
    );
}
