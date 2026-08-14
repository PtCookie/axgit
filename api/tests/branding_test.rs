//! Integration tests for `GET /api/v1/site/logo` and `GET /api/v1/site/favicon`
//! (docs/DECISIONS.md #81).

mod common;

use axgit::routes::build_router;
use axgit::state::AppState;
use axum::Router;
use axum::http::StatusCode;
use serde_json::Value;
use tempfile::TempDir;

/// A router with no repositories at all — neither endpoint is repo-scoped,
/// so an empty repo root is enough for every test here.
fn router_with_branding(logo: Option<&str>, favicon: Option<&str>) -> (TempDir, Router) {
    let repo_root = tempfile::tempdir().expect("failed to create repo root");
    let mut config = common::test_config(repo_root.path());
    config.logo = logo.map(str::to_owned);
    config.favicon = favicon.map(str::to_owned);
    let router = build_router(AppState::new(config));
    (repo_root, router)
}

#[tokio::test]
async fn logo_is_404_when_unset() {
    let (_repo_root, router) = router_with_branding(None, None);

    let (status, headers, body) = common::get_bytes_with_headers(router, "/api/v1/site/logo").await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(headers["content-type"], "application/json");
    let json: Value = serde_json::from_slice(&body).expect("body is not JSON");
    assert_eq!(json["error"]["code"], "not_found");
}

#[tokio::test]
async fn favicon_is_404_when_unset() {
    let (_repo_root, router) = router_with_branding(None, None);

    let (status, _, _) = common::get_bytes_with_headers(router, "/api/v1/site/favicon").await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn logo_is_404_when_configured_as_a_url() {
    let (_repo_root, router) = router_with_branding(Some("https://example.net/logo.svg"), None);

    let (status, _, _) = common::get_bytes_with_headers(router, "/api/v1/site/logo").await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a URL logo has nothing local to serve — the shell points straight at it"
    );
}

#[tokio::test]
async fn logo_serves_a_configured_file_with_its_content_type() {
    let logo_dir = tempfile::tempdir().expect("failed to create logo dir");
    let logo_path = logo_dir.path().join("logo.svg");
    std::fs::write(&logo_path, "<svg></svg>").expect("failed to write logo");
    let (_repo_root, router) = router_with_branding(Some(logo_path.to_str().unwrap()), None);

    let (status, headers, body) = common::get_bytes_with_headers(router, "/api/v1/site/logo").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "image/svg+xml");
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert_eq!(headers["cache-control"], "no-cache");
    assert!(headers.contains_key("etag"));
    assert_eq!(body, b"<svg></svg>");
}

#[tokio::test]
async fn favicon_serves_a_configured_file_with_its_content_type() {
    let dir = tempfile::tempdir().expect("failed to create favicon dir");
    let path = dir.path().join("favicon.png");
    std::fs::write(&path, [0x89, b'P', b'N', b'G']).expect("failed to write favicon");
    let (_repo_root, router) = router_with_branding(None, Some(path.to_str().unwrap()));

    let (status, headers, body) =
        common::get_bytes_with_headers(router, "/api/v1/site/favicon").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "image/png");
    assert_eq!(body, [0x89, b'P', b'N', b'G']);
}

#[tokio::test]
async fn logo_is_404_for_a_missing_file() {
    let (_repo_root, router) = router_with_branding(Some("/no/such/logo.svg"), None);

    let (status, _, _) = common::get_bytes_with_headers(router, "/api/v1/site/logo").await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn logo_is_404_for_an_unrecognized_extension() {
    let dir = tempfile::tempdir().expect("failed to create logo dir");
    let path = dir.path().join("logo.html");
    std::fs::write(&path, "<html></html>").expect("failed to write file");
    let (_repo_root, router) = router_with_branding(Some(path.to_str().unwrap()), None);

    let (status, _, _) = common::get_bytes_with_headers(router, "/api/v1/site/logo").await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "an unrecognized extension must never be served, even as an odd content-type"
    );
}

#[tokio::test]
async fn logo_etag_round_trips_through_if_none_match() {
    let logo_dir = tempfile::tempdir().expect("failed to create logo dir");
    let logo_path = logo_dir.path().join("logo.svg");
    std::fs::write(&logo_path, "<svg></svg>").expect("failed to write logo");

    let (_repo_root, router) = router_with_branding(Some(logo_path.to_str().unwrap()), None);
    let (status, headers, _) = common::get_bytes_with_headers(router, "/api/v1/site/logo").await;
    assert_eq!(status, StatusCode::OK);
    let etag = headers["etag"].to_str().unwrap().to_owned();

    let (_repo_root2, router2) = router_with_branding(Some(logo_path.to_str().unwrap()), None);
    let (status, _, body) = common::get_bytes_with_request_headers(
        router2,
        "/api/v1/site/logo",
        &[("if-none-match", &etag)],
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_MODIFIED,
        "unexpected response: {body:?}"
    );
}

/// Sanity check that the two endpoints are independent — configuring one
/// doesn't imply the other.
#[tokio::test]
async fn logo_and_favicon_are_configured_independently() {
    let dir = tempfile::tempdir().expect("failed to create dir");
    let logo_path = dir.path().join("logo.svg");
    std::fs::write(&logo_path, "<svg></svg>").expect("failed to write logo");
    let (_repo_root, router) = router_with_branding(Some(logo_path.to_str().unwrap()), None);

    let (logo_status, _, _) = common::get_bytes_with_headers(router, "/api/v1/site/logo").await;
    let (_repo_root2, router2) = router_with_branding(Some(logo_path.to_str().unwrap()), None);
    let (favicon_status, _, _) =
        common::get_bytes_with_headers(router2, "/api/v1/site/favicon").await;

    assert_eq!(logo_status, StatusCode::OK);
    assert_eq!(favicon_status, StatusCode::NOT_FOUND);
}
