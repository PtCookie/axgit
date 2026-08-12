mod common;

use common::router_for;

use std::path::Path;

use axgit::repo::sort::RepoOrder;
use axgit::routes::build_router;
use axgit::state::AppState;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tempfile::TempDir;
use tower::ServiceExt;

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
async fn list_repos_defaults_to_name_order_and_echoes_the_effective_sort() {
    let root = setup_fixtures();

    let json = list_repos(root.path()).await;

    assert_eq!(json["sort"], "name");
}

#[tokio::test]
async fn list_repos_sort_owner_places_a_missing_owner_last() {
    let root = setup_fixtures();

    let (status, json) =
        common::get_json(router_for(root.path()), "/api/v1/repos?sort=owner").await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");

    let names: Vec<&str> = json["repos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|repo| repo["name"].as_str().unwrap())
        .collect();
    // alpha has owner "PtCookie"; bravo/empty have none and fall back to the
    // name-ascending tiebreak, last regardless of direction.
    assert_eq!(names, ["alpha", "bravo", "empty"]);
    assert_eq!(json["sort"], "owner");
}

#[tokio::test]
async fn list_repos_sort_reversed_owner_still_places_a_missing_owner_last() {
    let root = setup_fixtures();

    let (status, json) =
        common::get_json(router_for(root.path()), "/api/v1/repos?sort=-owner").await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");

    let names: Vec<&str> = json["repos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|repo| repo["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["alpha", "bravo", "empty"]);
    assert_eq!(json["sort"], "-owner");
}

#[tokio::test]
async fn list_repos_sort_idle_defaults_to_descending_newest_first() {
    let root = setup_fixtures();

    let (status, json) = common::get_json(router_for(root.path()), "/api/v1/repos?sort=idle").await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");

    let names: Vec<&str> = json["repos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|repo| repo["name"].as_str().unwrap())
        .collect();
    // alpha: 2026-07-24 (agefile); bravo: FIXED_DATE = 2026-07-01 (authordate
    // fallback); empty: null, always last.
    assert_eq!(names, ["alpha", "bravo", "empty"]);
    assert_eq!(json["sort"], "idle");
}

#[tokio::test]
async fn list_repos_honours_the_configured_server_default_sort() {
    let root = setup_fixtures();

    let mut config = common::test_config(root.path());
    config.repository_sort = RepoOrder::parse("owner").unwrap();
    let router = build_router(AppState::new(config));

    let (status, json) = common::get_json(router, "/api/v1/repos").await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");
    assert_eq!(json["sort"], "owner");
}

#[tokio::test]
async fn list_repos_rejects_an_unknown_sort() {
    let root = setup_fixtures();

    let (status, json) =
        common::get_json(router_for(root.path()), "/api/v1/repos?sort=bogus").await;

    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("invalid_param")),
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
