//! Integration tests for `GET /api/v1/site` (docs/DECISIONS.md #70).

mod common;

use std::path::Path;

use axgit::routes::build_router;
use axgit::state::AppState;
use axum::Router;
use axum::http::StatusCode;
use serde_json::Value;
use tempfile::TempDir;

/// A router with no repositories at all — `/site` is not repo-scoped, so an
/// empty repo root is enough for every test here.
fn router_with_site(
    root_title: Option<&str>,
    root_desc: Option<&str>,
    root_readme: Option<&Path>,
) -> (TempDir, Router) {
    let repo_root = tempfile::tempdir().expect("failed to create repo root");
    let mut config = common::test_config(repo_root.path());
    config.root_title = root_title.map(str::to_owned);
    config.root_desc = root_desc.map(str::to_owned);
    config.root_readme = root_readme.map(Path::to_owned);
    let router = build_router(AppState::new(config));
    (repo_root, router)
}

async fn get_ok(router: Router) -> Value {
    let (status, json) = common::get_json(router, "/api/v1/site").await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");
    json
}

#[tokio::test]
async fn site_falls_back_to_defaults_when_everything_is_unset() {
    let (_repo_root, router) = router_with_site(None, None, None);

    let json = get_ok(router).await;

    assert_eq!(
        json,
        serde_json::json!({ "title": "Axgit", "description": null, "readme": null })
    );
}

#[tokio::test]
async fn site_reports_the_configured_title_and_description() {
    let (_repo_root, router) =
        router_with_site(Some("PtCookie Git"), Some("Self-hosted repositories"), None);

    let json = get_ok(router).await;

    assert_eq!(
        (
            json["title"].as_str(),
            json["description"].as_str(),
            json["readme"].clone()
        ),
        (
            Some("PtCookie Git"),
            Some("Self-hosted repositories"),
            Value::Null
        ),
    );
}

#[tokio::test]
async fn site_reads_a_configured_markdown_readme() {
    let readme_dir = tempfile::tempdir().expect("failed to create readme dir");
    let readme_path = readme_dir.path().join("ROOT_README.md");
    std::fs::write(&readme_path, "# Welcome\n\nSee the repos below.\n")
        .expect("failed to write readme");
    let (_repo_root, router) = router_with_site(None, None, Some(&readme_path));

    let json = get_ok(router).await;

    assert_eq!(
        (
            json["readme"]["format"].as_str(),
            json["readme"]["content"].as_str()
        ),
        (
            Some("markdown"),
            Some("# Welcome\n\nSee the repos below.\n")
        ),
    );
}

#[tokio::test]
async fn site_guesses_plain_format_for_a_non_markdown_extension() {
    let readme_dir = tempfile::tempdir().expect("failed to create readme dir");
    let readme_path = readme_dir.path().join("ROOT_README.txt");
    std::fs::write(&readme_path, "Plain text welcome.\n").expect("failed to write readme");
    let (_repo_root, router) = router_with_site(None, None, Some(&readme_path));

    let json = get_ok(router).await;

    assert_eq!(json["readme"]["format"], "plain");
}

#[tokio::test]
async fn site_readme_is_null_when_the_configured_file_is_missing() {
    let (_repo_root, router) = router_with_site(None, None, Some(Path::new("/no/such/README.md")));

    let json = get_ok(router).await;

    assert_eq!(json["readme"], Value::Null);
}

#[tokio::test]
async fn site_readme_is_null_when_the_configured_file_is_too_large() {
    let readme_dir = tempfile::tempdir().expect("failed to create readme dir");
    let readme_path = readme_dir.path().join("ROOT_README.md");
    std::fs::write(&readme_path, vec![b'a'; 513 * 1024]).expect("failed to write readme");
    let (_repo_root, router) = router_with_site(None, None, Some(&readme_path));

    let json = get_ok(router).await;

    assert_eq!(json["readme"], Value::Null);
    // The over-limit readme doesn't take title/description down with it.
    assert_eq!(json["title"], "Axgit");
}

#[tokio::test]
async fn site_etag_round_trips_through_if_none_match() {
    let (_repo_root, router) = router_with_site(Some("PtCookie Git"), None, None);

    let (status, headers, _) = common::get_json_with_headers(router, "/api/v1/site").await;
    assert_eq!(status, StatusCode::OK);
    let etag = headers["etag"].to_str().unwrap().to_owned();

    let (_repo_root2, router2) = router_with_site(Some("PtCookie Git"), None, None);
    let (status, _, body) = common::get_bytes_with_request_headers(
        router2,
        "/api/v1/site",
        &[("if-none-match", &etag)],
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_MODIFIED,
        "unexpected response: {body:?}"
    );
}
