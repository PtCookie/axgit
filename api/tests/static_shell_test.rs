//! Integration tests for the static page-shell fallback (docs/DECISIONS.md #17).
//!
//! Astro static builds cannot enumerate `/{repo}/...` routes at build time,
//! so `web/src/pages/[repo]/` is prerendered once under a reserved
//! placeholder param (`__repo__`) and the server maps request *shapes* onto
//! those shell files: serve real static files when present, `/` and
//! `/{repo}` and `/{repo}/refs` onto their respective shells, and anything
//! else onto `404.html` with a real 404 status. The `/api/v1` router must
//! not inherit that fallback — unmatched API paths keep answering with the
//! normal JSON error envelope — and Smart HTTP / Swagger UI must take
//! precedence over it as real routes.

mod common;

use axum::http::StatusCode;
use serde_json::Value;
use tempfile::TempDir;

use common::{get_bytes_with_headers, router_for, router_with_static};

const INDEX_HTML: &str = "<!doctype html><html><body>INDEX</body></html>";
const REPO_HTML: &str = "<!doctype html><html><body>REPO SUMMARY</body></html>";
const REFS_HTML: &str = "<!doctype html><html><body>REPO REFS</body></html>";
const LOG_HTML: &str = "<!doctype html><html><body>REPO LOG</body></html>";
const COMMIT_HTML: &str = "<!doctype html><html><body>REPO COMMIT</body></html>";
const TREE_HTML: &str = "<!doctype html><html><body>REPO TREE</body></html>";
const BLOB_HTML: &str = "<!doctype html><html><body>REPO BLOB</body></html>";
const BLAME_HTML: &str = "<!doctype html><html><body>REPO BLAME</body></html>";
const NOT_FOUND_HTML: &str = "<!doctype html><html><body>NOT FOUND</body></html>";
const ASSET_JS: &str = "console.log('app');";

/// A repo root with one repository, and a static dir with the nine page
/// shells (`index.html`, `__repo__/index.html`, `__repo__/refs/index.html`,
/// `__repo__/log/index.html`, `__repo__/commit/index.html`,
/// `__repo__/tree/index.html`, `__repo__/blob/index.html`,
/// `__repo__/blame/index.html`, `404.html`) plus a real asset under
/// `_astro/`.
fn setup_fixtures() -> (TempDir, TempDir) {
    let repo_root = tempfile::tempdir().expect("failed to create repo root");
    common::create_bare_repo(repo_root.path(), "git-compose.git");

    let static_dir = tempfile::tempdir().expect("failed to create static dir");
    std::fs::write(static_dir.path().join("index.html"), INDEX_HTML).unwrap();
    std::fs::write(static_dir.path().join("404.html"), NOT_FOUND_HTML).unwrap();
    std::fs::create_dir_all(static_dir.path().join("__repo__/refs")).unwrap();
    std::fs::create_dir_all(static_dir.path().join("__repo__/log")).unwrap();
    std::fs::create_dir_all(static_dir.path().join("__repo__/commit")).unwrap();
    std::fs::create_dir_all(static_dir.path().join("__repo__/tree")).unwrap();
    std::fs::create_dir_all(static_dir.path().join("__repo__/blob")).unwrap();
    std::fs::create_dir_all(static_dir.path().join("__repo__/blame")).unwrap();
    std::fs::write(static_dir.path().join("__repo__/index.html"), REPO_HTML).unwrap();
    std::fs::write(
        static_dir.path().join("__repo__/refs/index.html"),
        REFS_HTML,
    )
    .unwrap();
    std::fs::write(static_dir.path().join("__repo__/log/index.html"), LOG_HTML).unwrap();
    std::fs::write(
        static_dir.path().join("__repo__/commit/index.html"),
        COMMIT_HTML,
    )
    .unwrap();
    std::fs::write(
        static_dir.path().join("__repo__/tree/index.html"),
        TREE_HTML,
    )
    .unwrap();
    std::fs::write(
        static_dir.path().join("__repo__/blob/index.html"),
        BLOB_HTML,
    )
    .unwrap();
    std::fs::write(
        static_dir.path().join("__repo__/blame/index.html"),
        BLAME_HTML,
    )
    .unwrap();
    std::fs::create_dir_all(static_dir.path().join("_astro")).unwrap();
    std::fs::write(static_dir.path().join("_astro/app.js"), ASSET_JS).unwrap();

    (repo_root, static_dir)
}

async fn assert_html_shell(
    router: axum::Router,
    uri: &str,
    expected_status: StatusCode,
    expected_body: &str,
) {
    let (status, headers, body) = get_bytes_with_headers(router, uri).await;

    assert_eq!(status, expected_status, "uri {uri}");
    assert!(
        headers["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html"),
        "uri {uri} did not serve HTML"
    );
    assert_eq!(String::from_utf8(body).unwrap(), expected_body, "uri {uri}");
}

#[tokio::test]
async fn root_should_serve_the_index_shell() {
    let (repo_root, static_dir) = setup_fixtures();
    let router = router_with_static(repo_root.path(), static_dir.path());

    assert_html_shell(router, "/", StatusCode::OK, INDEX_HTML).await;
}

#[tokio::test]
async fn repo_paths_should_serve_the_repo_shell() {
    let (repo_root, static_dir) = setup_fixtures();

    for uri in ["/git-compose", "/git-compose/", "/my%20repo"] {
        let router = router_with_static(repo_root.path(), static_dir.path());
        assert_html_shell(router, uri, StatusCode::OK, REPO_HTML).await;
    }
}

#[tokio::test]
async fn repo_refs_paths_should_serve_the_refs_shell() {
    let (repo_root, static_dir) = setup_fixtures();

    for uri in ["/git-compose/refs", "/git-compose/refs/"] {
        let router = router_with_static(repo_root.path(), static_dir.path());
        assert_html_shell(router, uri, StatusCode::OK, REFS_HTML).await;
    }
}

#[tokio::test]
async fn repo_log_paths_should_serve_the_log_shell() {
    let (repo_root, static_dir) = setup_fixtures();

    for uri in ["/git-compose/log", "/git-compose/log/"] {
        let router = router_with_static(repo_root.path(), static_dir.path());
        assert_html_shell(router, uri, StatusCode::OK, LOG_HTML).await;
    }
}

#[tokio::test]
async fn repo_commit_paths_should_serve_the_commit_shell() {
    let (repo_root, static_dir) = setup_fixtures();

    for uri in ["/git-compose/commit/abc123", "/git-compose/commit/abc123/"] {
        let router = router_with_static(repo_root.path(), static_dir.path());
        assert_html_shell(router, uri, StatusCode::OK, COMMIT_HTML).await;
    }
}

#[tokio::test]
async fn repo_tree_paths_should_serve_the_tree_shell() {
    let (repo_root, static_dir) = setup_fixtures();

    for uri in [
        "/git-compose/tree",
        "/git-compose/tree/",
        "/git-compose/tree/src",
        "/git-compose/tree/src/lib",
    ] {
        let router = router_with_static(repo_root.path(), static_dir.path());
        assert_html_shell(router, uri, StatusCode::OK, TREE_HTML).await;
    }
}

#[tokio::test]
async fn repo_blob_paths_should_serve_the_blob_shell() {
    let (repo_root, static_dir) = setup_fixtures();

    for uri in [
        "/git-compose/blob/src/main.rs",
        "/git-compose/blob/README.md",
    ] {
        let router = router_with_static(repo_root.path(), static_dir.path());
        assert_html_shell(router, uri, StatusCode::OK, BLOB_HTML).await;
    }
}

#[tokio::test]
async fn repo_blame_paths_should_serve_the_blame_shell() {
    let (repo_root, static_dir) = setup_fixtures();

    for uri in [
        "/git-compose/blame/src/main.rs",
        "/git-compose/blame/README.md",
    ] {
        let router = router_with_static(repo_root.path(), static_dir.path());
        assert_html_shell(router, uri, StatusCode::OK, BLAME_HTML).await;
    }
}

#[tokio::test]
async fn unknown_paths_should_serve_the_404_shell_with_a_404_status() {
    let (repo_root, static_dir) = setup_fixtures();

    for uri in [
        "/git-compose/blob",
        "/git-compose/blob/",
        "/git-compose/blame",
        "/git-compose/blame/",
        "/git-compose/commit",
        "/git-compose/commit/abc123/extra",
        "/git-compose/stats/extra",
        "/a/b/c",
    ] {
        let router = router_with_static(repo_root.path(), static_dir.path());
        assert_html_shell(router, uri, StatusCode::NOT_FOUND, NOT_FOUND_HTML).await;
    }
}

#[tokio::test]
async fn real_static_assets_should_be_served_instead_of_a_shell() {
    let (repo_root, static_dir) = setup_fixtures();
    let router = router_with_static(repo_root.path(), static_dir.path());

    let (status, _, body) = get_bytes_with_headers(router, "/_astro/app.js").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(String::from_utf8(body).unwrap(), ASSET_JS);
}

#[tokio::test]
async fn unmatched_api_paths_should_return_json_not_found_not_a_shell() {
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
async fn smart_http_routes_should_take_precedence_over_the_shell() {
    let (repo_root, static_dir) = setup_fixtures();
    let router = router_with_static(repo_root.path(), static_dir.path());

    let (status, headers, _) =
        get_bytes_with_headers(router, "/git-compose.git/info/refs?service=git-upload-pack").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers["content-type"],
        "application/x-git-upload-pack-advertisement"
    );
}

#[tokio::test]
async fn swagger_ui_should_take_precedence_over_the_shell() {
    let (repo_root, static_dir) = setup_fixtures();
    let router = router_with_static(repo_root.path(), static_dir.path());

    let (status, headers, _) = get_bytes_with_headers(router, "/swagger-ui").await;

    // utoipa-swagger-ui redirects `/swagger-ui` to `/swagger-ui/`; either way
    // it must not be the page shell.
    assert!(
        status == StatusCode::OK || status.is_redirection(),
        "status was {status}"
    );
    if status == StatusCode::OK {
        assert!(
            headers["content-type"]
                .to_str()
                .unwrap()
                .starts_with("text/html"),
        );
    }
}

#[tokio::test]
async fn without_a_static_dir_unmatched_paths_should_still_404() {
    let repo_root = tempfile::tempdir().expect("failed to create repo root");
    let router = router_for(repo_root.path());

    let (status, _, _) = get_bytes_with_headers(router, "/nope").await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}
