//! Integration tests for the SPA shell fallback (docs/DECISIONS.md #16).
//!
//! Astro static builds cannot enumerate `/{repo}/...` routes at build time,
//! so the server serves real static files when present and falls back to
//! `index.html` otherwise, letting the client-side router take over. The
//! `/api/v1` router must not inherit that fallback — unmatched API paths
//! keep answering with the normal JSON error envelope.

mod common;

use axum::http::StatusCode;
use serde_json::Value;
use tempfile::TempDir;

use common::{get_bytes_with_headers, router_for, router_with_static};

const SHELL_HTML: &str = "<!doctype html><html><body>SHELL</body></html>";
const ASSET_JS: &str = "console.log('app');";

/// A repo root with one repository, and a static dir with an `index.html`
/// shell plus a real asset under `_astro/`.
fn setup_fixtures() -> (TempDir, TempDir) {
    let repo_root = tempfile::tempdir().expect("failed to create repo root");
    common::create_bare_repo(repo_root.path(), "git-compose.git");

    let static_dir = tempfile::tempdir().expect("failed to create static dir");
    std::fs::write(static_dir.path().join("index.html"), SHELL_HTML).unwrap();
    std::fs::create_dir_all(static_dir.path().join("_astro")).unwrap();
    std::fs::write(static_dir.path().join("_astro/app.js"), ASSET_JS).unwrap();

    (repo_root, static_dir)
}

#[tokio::test]
async fn root_should_serve_the_shell() {
    let (repo_root, static_dir) = setup_fixtures();
    let router = router_with_static(repo_root.path(), static_dir.path());

    let (status, headers, body) = get_bytes_with_headers(router, "/").await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        headers["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    assert_eq!(String::from_utf8(body).unwrap(), SHELL_HTML);
}

#[tokio::test]
async fn unmatched_repo_subpaths_should_fall_back_to_the_shell() {
    let (repo_root, static_dir) = setup_fixtures();

    for uri in ["/git-compose", "/git-compose/refs", "/git-compose/log"] {
        let router = router_with_static(repo_root.path(), static_dir.path());
        let (status, headers, body) = get_bytes_with_headers(router, uri).await;

        assert_eq!(status, StatusCode::OK, "uri {uri} did not return 200");
        assert!(
            headers["content-type"]
                .to_str()
                .unwrap()
                .starts_with("text/html"),
            "uri {uri} did not serve HTML"
        );
        assert_eq!(String::from_utf8(body).unwrap(), SHELL_HTML, "uri {uri}");
    }
}

#[tokio::test]
async fn real_static_assets_should_be_served_instead_of_the_shell() {
    let (repo_root, static_dir) = setup_fixtures();
    let router = router_with_static(repo_root.path(), static_dir.path());

    let (status, _, body) = get_bytes_with_headers(router, "/_astro/app.js").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(String::from_utf8(body).unwrap(), ASSET_JS);
}

#[tokio::test]
async fn unmatched_api_paths_should_return_json_not_found_not_the_shell() {
    let (repo_root, static_dir) = setup_fixtures();

    for uri in ["/api/v1/bogus", "/api/v1/repos/git-compose/bogus"] {
        let router = router_with_static(repo_root.path(), static_dir.path());
        let (status, headers, body) = get_bytes_with_headers(router, uri).await;

        assert_eq!(status, StatusCode::NOT_FOUND, "uri {uri}");
        assert_eq!(headers["content-type"], "application/json", "uri {uri}");
        let json: Value = serde_json::from_slice(&body).expect("body is not JSON");
        assert_eq!(json["error"]["code"], "not_found", "uri {uri}");
    }
}

#[tokio::test]
async fn without_a_static_dir_unmatched_paths_should_still_404() {
    let repo_root = tempfile::tempdir().expect("failed to create repo root");
    let router = router_for(repo_root.path());

    let (status, _, _) = get_bytes_with_headers(router, "/nope").await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}
