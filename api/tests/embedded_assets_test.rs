//! Integration tests for the `embed-web` feature (docs/DECISIONS.md #74) —
//! runs against the real `web/dist`, so `pnpm --filter web build` must have
//! produced it before this test binary compiles/runs (same precondition the
//! feature itself carries). Compiled only when the feature is enabled: the
//! default build has no `web/dist` to embed at all.
#![cfg(feature = "embed-web")]

mod common;

use axum::http::StatusCode;
use tempfile::TempDir;

use common::{
    get_bytes_with_headers, get_bytes_with_request_headers, router_for, router_with_site,
};

fn empty_repo_root() -> TempDir {
    tempfile::tempdir().expect("failed to create repo root")
}

#[tokio::test]
async fn root_should_serve_the_real_embedded_index() {
    let repo_root = empty_repo_root();
    let router = router_for(repo_root.path());

    let (status, headers, body) = get_bytes_with_headers(router, "/").await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        headers["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html"),
    );
    let body = String::from_utf8_lossy(&body);
    assert!(body.contains("<title>Axgit</title>"), "body was {body:?}");
}

#[tokio::test]
async fn a_real_static_asset_should_be_served_with_a_guessed_content_type() {
    let repo_root = empty_repo_root();
    let router = router_for(repo_root.path());

    let (status, headers, body) = get_bytes_with_headers(router, "/robots.txt").await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        headers["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/plain"),
        "content-type was {:?}",
        headers["content-type"]
    );
    assert!(String::from_utf8_lossy(&body).contains("User-agent: *"));
}

#[tokio::test]
async fn an_svg_asset_should_be_served_with_the_svg_content_type() {
    let repo_root = empty_repo_root();
    let router = router_for(repo_root.path());

    let (status, headers, _) = get_bytes_with_headers(router, "/favicon.svg").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "image/svg+xml");
}

#[tokio::test]
async fn a_repo_route_shape_should_serve_the_matching_shell() {
    let repo_root = empty_repo_root();
    let router = router_for(repo_root.path());

    let (status, headers, body) = get_bytes_with_headers(router, "/git-compose/log").await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        headers["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html"),
    );
    let body = String::from_utf8_lossy(&body);
    assert!(
        body.contains("<title>Log — Axgit</title>"),
        "body was {body:?}"
    );
}

#[tokio::test]
async fn an_unmatched_shape_should_serve_the_404_shell_with_a_real_404() {
    let repo_root = empty_repo_root();
    let router = router_for(repo_root.path());

    let (status, _, body) = get_bytes_with_headers(router, "/a/b/c").await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    let body = String::from_utf8_lossy(&body);
    assert!(
        body.contains("<title>Page not found — Axgit</title>"),
        "body was {body:?}"
    );
}

#[tokio::test]
async fn a_dot_dot_path_should_not_escape_the_embedded_build() {
    let repo_root = empty_repo_root();
    let router = router_for(repo_root.path());

    // No embedded key contains `..`, so this can only ever fall through to
    // the shell fallback (a 3-segment unmatched shape → the 404 shell), never
    // reach outside the embedded build.
    let (status, _, _) = get_bytes_with_headers(router, "/../../../etc/passwd").await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_asset_etag_should_round_trip_through_if_none_match() {
    let repo_root = empty_repo_root();
    let router = router_for(repo_root.path());

    let (status, headers, _) = get_bytes_with_headers(router, "/robots.txt").await;
    assert_eq!(status, StatusCode::OK);
    let etag = headers["etag"].to_str().unwrap().to_owned();

    let router = router_for(repo_root.path());
    let (status, headers, body) =
        get_bytes_with_request_headers(router, "/robots.txt", &[("if-none-match", &etag)]).await;

    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert_eq!(headers["etag"].to_str().unwrap(), etag);
    assert!(body.is_empty());
}

#[tokio::test]
async fn a_configured_static_dir_should_override_the_embedded_build() {
    let repo_root = empty_repo_root();
    let static_dir = tempfile::tempdir().expect("failed to create static dir");
    std::fs::write(
        static_dir.path().join("index.html"),
        "<!doctype html><html><head></head><body>OVERRIDE</body></html>",
    )
    .unwrap();
    std::fs::write(static_dir.path().join("404.html"), "not found").unwrap();

    let router = common::router_with_static(repo_root.path(), static_dir.path());
    let (status, _, body) = get_bytes_with_headers(router, "/").await;

    assert_eq!(status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&body).contains("OVERRIDE"));
}

#[tokio::test]
async fn the_embedded_shell_should_still_carry_configured_site_meta() {
    let repo_root = empty_repo_root();
    let router = router_with_site(repo_root.path(), Some("PtCookie Git"), None);

    let (status, _, body) = get_bytes_with_headers(router, "/").await;

    assert_eq!(status, StatusCode::OK);
    let body = String::from_utf8_lossy(&body);
    assert!(
        body.contains(r#"<meta name="axgit:site-title" content="PtCookie Git">"#),
        "body was {body:?}"
    );
}
