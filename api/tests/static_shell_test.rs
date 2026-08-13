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

use common::{
    get_bytes_with_headers, router_for, router_with_static, router_with_static_and_clone_base,
    router_with_static_and_site,
};

// Each fixture carries a real `<head></head>` so the injected `<link>`s
// (docs/DECISIONS.md #63) have an anchor, and each is checked below via a
// body-only marker rather than the whole string — an exact match would break
// on every repo shell once the head is no longer empty.
const INDEX_HTML: &str = "<!doctype html><html><head></head><body>INDEX</body></html>";
const REPO_HTML: &str = "<!doctype html><html><head></head><body>REPO SUMMARY</body></html>";
const REFS_HTML: &str = "<!doctype html><html><head></head><body>REPO REFS</body></html>";
const LOG_HTML: &str = "<!doctype html><html><head></head><body>REPO LOG</body></html>";
const COMMIT_HTML: &str = "<!doctype html><html><head></head><body>REPO COMMIT</body></html>";
const OBJECT_HTML: &str = "<!doctype html><html><head></head><body>REPO OBJECT</body></html>";
const TREE_HTML: &str = "<!doctype html><html><head></head><body>REPO TREE</body></html>";
const BLOB_HTML: &str = "<!doctype html><html><head></head><body>REPO BLOB</body></html>";
const BLAME_HTML: &str = "<!doctype html><html><head></head><body>REPO BLAME</body></html>";
const TAG_HTML: &str = "<!doctype html><html><head></head><body>REPO TAG</body></html>";
const NOT_FOUND_HTML: &str = "<!doctype html><html><head></head><body>NOT FOUND</body></html>";
const ASSET_JS: &str = "console.log('app');";

const INDEX_MARKER: &str = "<body>INDEX</body>";
const REPO_MARKER: &str = "<body>REPO SUMMARY</body>";
const REFS_MARKER: &str = "<body>REPO REFS</body>";
const LOG_MARKER: &str = "<body>REPO LOG</body>";
const COMMIT_MARKER: &str = "<body>REPO COMMIT</body>";
const OBJECT_MARKER: &str = "<body>REPO OBJECT</body>";
const TREE_MARKER: &str = "<body>REPO TREE</body>";
const BLOB_MARKER: &str = "<body>REPO BLOB</body>";
const BLAME_MARKER: &str = "<body>REPO BLAME</body>";
const TAG_MARKER: &str = "<body>REPO TAG</body>";
const NOT_FOUND_MARKER: &str = "<body>NOT FOUND</body>";

/// A repo root with one repository, and a static dir with the eleven page
/// shells (`index.html`, `__repo__/index.html`, `__repo__/refs/index.html`,
/// `__repo__/log/index.html`, `__repo__/commit/index.html`,
/// `__repo__/object/index.html`, `__repo__/tree/index.html`,
/// `__repo__/blob/index.html`, `__repo__/blame/index.html`,
/// `__repo__/tag/index.html`, `404.html`) plus a real asset under `_astro/`.
fn setup_fixtures() -> (TempDir, TempDir) {
    let repo_root = tempfile::tempdir().expect("failed to create repo root");
    common::create_bare_repo(repo_root.path(), "git-compose.git");

    let static_dir = tempfile::tempdir().expect("failed to create static dir");
    std::fs::write(static_dir.path().join("index.html"), INDEX_HTML).unwrap();
    std::fs::write(static_dir.path().join("404.html"), NOT_FOUND_HTML).unwrap();
    std::fs::create_dir_all(static_dir.path().join("__repo__/refs")).unwrap();
    std::fs::create_dir_all(static_dir.path().join("__repo__/log")).unwrap();
    std::fs::create_dir_all(static_dir.path().join("__repo__/commit")).unwrap();
    std::fs::create_dir_all(static_dir.path().join("__repo__/object")).unwrap();
    std::fs::create_dir_all(static_dir.path().join("__repo__/tree")).unwrap();
    std::fs::create_dir_all(static_dir.path().join("__repo__/blob")).unwrap();
    std::fs::create_dir_all(static_dir.path().join("__repo__/blame")).unwrap();
    std::fs::create_dir_all(static_dir.path().join("__repo__/tag")).unwrap();
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
        static_dir.path().join("__repo__/object/index.html"),
        OBJECT_HTML,
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
    std::fs::write(static_dir.path().join("__repo__/tag/index.html"), TAG_HTML).unwrap();
    std::fs::create_dir_all(static_dir.path().join("_astro")).unwrap();
    std::fs::write(static_dir.path().join("_astro/app.js"), ASSET_JS).unwrap();

    (repo_root, static_dir)
}

/// Asserts the served body *contains* `expected_marker` rather than equals it
/// outright — repository shells now carry injected `<link>`s
/// (docs/DECISIONS.md #63), so an exact match would break on the marker
/// fixtures above.
async fn assert_html_shell(
    router: axum::Router,
    uri: &str,
    expected_status: StatusCode,
    expected_marker: &str,
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
    let body = String::from_utf8(body).unwrap();
    assert!(
        body.contains(expected_marker),
        "uri {uri} body {body:?} did not contain {expected_marker:?}"
    );
}

#[tokio::test]
async fn root_should_serve_the_index_shell() {
    let (repo_root, static_dir) = setup_fixtures();
    let router = router_with_static(repo_root.path(), static_dir.path());

    assert_html_shell(router, "/", StatusCode::OK, INDEX_MARKER).await;
}

#[tokio::test]
async fn repo_paths_should_serve_the_repo_shell() {
    let (repo_root, static_dir) = setup_fixtures();

    for uri in ["/git-compose", "/git-compose/", "/my%20repo"] {
        let router = router_with_static(repo_root.path(), static_dir.path());
        assert_html_shell(router, uri, StatusCode::OK, REPO_MARKER).await;
    }
}

#[tokio::test]
async fn repo_refs_paths_should_serve_the_refs_shell() {
    let (repo_root, static_dir) = setup_fixtures();

    for uri in ["/git-compose/refs", "/git-compose/refs/"] {
        let router = router_with_static(repo_root.path(), static_dir.path());
        assert_html_shell(router, uri, StatusCode::OK, REFS_MARKER).await;
    }
}

#[tokio::test]
async fn repo_log_paths_should_serve_the_log_shell() {
    let (repo_root, static_dir) = setup_fixtures();

    for uri in ["/git-compose/log", "/git-compose/log/"] {
        let router = router_with_static(repo_root.path(), static_dir.path());
        assert_html_shell(router, uri, StatusCode::OK, LOG_MARKER).await;
    }
}

#[tokio::test]
async fn repo_commit_paths_should_serve_the_commit_shell() {
    let (repo_root, static_dir) = setup_fixtures();

    for uri in ["/git-compose/commit/abc123", "/git-compose/commit/abc123/"] {
        let router = router_with_static(repo_root.path(), static_dir.path());
        assert_html_shell(router, uri, StatusCode::OK, COMMIT_MARKER).await;
    }
}

#[tokio::test]
async fn repo_object_paths_should_serve_the_object_shell() {
    let (repo_root, static_dir) = setup_fixtures();

    for uri in ["/git-compose/object/abc123", "/git-compose/object/abc123/"] {
        let router = router_with_static(repo_root.path(), static_dir.path());
        assert_html_shell(router, uri, StatusCode::OK, OBJECT_MARKER).await;
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
        assert_html_shell(router, uri, StatusCode::OK, TREE_MARKER).await;
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
        assert_html_shell(router, uri, StatusCode::OK, BLOB_MARKER).await;
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
        assert_html_shell(router, uri, StatusCode::OK, BLAME_MARKER).await;
    }
}

#[tokio::test]
async fn repo_tag_paths_should_serve_the_tag_shell() {
    let (repo_root, static_dir) = setup_fixtures();

    for uri in ["/git-compose/tag/v1.0.0", "/git-compose/tag/release/1.0"] {
        let router = router_with_static(repo_root.path(), static_dir.path());
        assert_html_shell(router, uri, StatusCode::OK, TAG_MARKER).await;
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
        "/git-compose/object",
        "/git-compose/object/",
        "/git-compose/object/abc123/extra",
        "/git-compose/stats/extra",
        "/git-compose/tag",
        "/git-compose/tag/",
        "/a/b/c",
    ] {
        let router = router_with_static(repo_root.path(), static_dir.path());
        assert_html_shell(router, uri, StatusCode::NOT_FOUND, NOT_FOUND_MARKER).await;
    }
}

#[tokio::test]
async fn repo_shells_should_carry_both_feed_discovery_links() {
    let (repo_root, static_dir) = setup_fixtures();

    for uri in ["/git-compose", "/git-compose/log", "/git-compose/tree/src"] {
        let router = router_with_static(repo_root.path(), static_dir.path());
        let (_, _, body) = get_bytes_with_headers(router, uri).await;
        let body = String::from_utf8(body).unwrap();

        assert!(
            body.contains(
                "<link rel=\"alternate\" type=\"application/atom+xml\" title=\"Recent commits\" href=\"/api/v1/repos/git-compose/feed.atom\">"
            ),
            "uri {uri} missing the default feed link: {body:?}"
        );
        assert!(
            body.contains(
                "<link rel=\"alternate\" type=\"application/atom+xml\" title=\"Recent commits (all refs)\" href=\"/api/v1/repos/git-compose/feed.atom?all=1\">"
            ),
            "uri {uri} missing the all-refs feed link: {body:?}"
        );
        assert!(
            !body.contains("vcs-git"),
            "uri {uri} unexpectedly carries vcs-git without a configured clone_url_base"
        );
    }
}

#[tokio::test]
async fn repo_shells_should_carry_the_vcs_git_link_when_a_clone_base_is_configured() {
    let (repo_root, static_dir) = setup_fixtures();
    let router = router_with_static_and_clone_base(
        repo_root.path(),
        static_dir.path(),
        "https://git.example.net",
    );

    let (_, _, body) = get_bytes_with_headers(router, "/git-compose").await;
    let body = String::from_utf8(body).unwrap();

    assert!(body.contains(
        "<link rel=\"vcs-git\" title=\"Git repository\" href=\"https://git.example.net/git-compose.git\">"
    ));
}

#[tokio::test]
async fn non_repo_shells_should_carry_no_head_links() {
    let (repo_root, static_dir) = setup_fixtures();

    for uri in ["/", "/git-compose/bogus"] {
        let router = router_with_static_and_clone_base(
            repo_root.path(),
            static_dir.path(),
            "https://git.example.net",
        );
        let (_, _, body) = get_bytes_with_headers(router, uri).await;
        let body = String::from_utf8(body).unwrap();

        assert!(
            !body.contains("<link rel=\"alternate\""),
            "uri {uri} unexpectedly carries a feed link: {body:?}"
        );
        assert!(
            !body.contains("vcs-git"),
            "uri {uri} unexpectedly carries a vcs-git link: {body:?}"
        );
    }
}

#[tokio::test]
async fn every_shell_carries_the_configured_site_meta() {
    let (repo_root, static_dir) = setup_fixtures();

    for uri in ["/", "/git-compose", "/git-compose/bogus"] {
        let router = router_with_static_and_site(
            repo_root.path(),
            static_dir.path(),
            Some("PtCookie Git"),
            Some("Self-hosted repositories"),
        );
        let (_, _, body) = get_bytes_with_headers(router, uri).await;
        let body = String::from_utf8(body).unwrap();

        assert!(
            body.contains("<meta name=\"axgit:site-title\" content=\"PtCookie Git\">"),
            "uri {uri} missing the site-title meta: {body:?}"
        );
        assert!(
            body.contains("<meta name=\"axgit:site-desc\" content=\"Self-hosted repositories\">"),
            "uri {uri} missing the site-desc meta: {body:?}"
        );
    }
}

#[tokio::test]
async fn no_shell_carries_site_meta_when_unconfigured() {
    let (repo_root, static_dir) = setup_fixtures();

    for uri in ["/", "/git-compose", "/git-compose/bogus"] {
        let router = router_with_static(repo_root.path(), static_dir.path());
        let (_, _, body) = get_bytes_with_headers(router, uri).await;
        let body = String::from_utf8(body).unwrap();

        assert!(
            !body.contains("axgit:site-"),
            "uri {uri} unexpectedly carries site meta: {body:?}"
        );
    }
}

#[tokio::test]
async fn site_meta_and_repo_head_links_coexist_on_a_repo_shell() {
    let (repo_root, static_dir) = setup_fixtures();
    let mut config = common::test_config(repo_root.path());
    config.static_dir = Some(static_dir.path().to_owned());
    config.root_title = Some("PtCookie Git".to_owned());
    config.clone_url_base = Some("https://git.example.net".to_owned());
    let router = axgit::routes::build_router(axgit::state::AppState::new(config));

    let (_, _, body) = get_bytes_with_headers(router, "/git-compose").await;
    let body = String::from_utf8(body).unwrap();

    assert!(body.contains("<meta name=\"axgit:site-title\" content=\"PtCookie Git\">"));
    assert!(body.contains(
        "<link rel=\"vcs-git\" title=\"Git repository\" href=\"https://git.example.net/git-compose.git\">"
    ));
}

#[tokio::test]
async fn real_static_assets_should_be_served_instead_of_a_shell() {
    let (repo_root, static_dir) = setup_fixtures();
    let router = router_with_static(repo_root.path(), static_dir.path());

    let (status, _, body) = get_bytes_with_headers(router, "/_astro/app.js").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(String::from_utf8(body).unwrap(), ASSET_JS);
}

// docs/DECISIONS.md #77: `_astro/*` is where Astro puts every content-hashed
// asset, so it's safe to mark immutable — checked against the three cases
// that must disagree: a real hashed asset, a shell (not under the prefix at
// all), and a `_astro/*` path with no matching file (falls through to the
// 404 shell, which must never inherit the immutable header).
#[tokio::test]
async fn a_hashed_asset_should_get_an_immutable_cache_control() {
    let (repo_root, static_dir) = setup_fixtures();
    let router = router_with_static(repo_root.path(), static_dir.path());

    let (status, headers, _) = get_bytes_with_headers(router, "/_astro/app.js").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers["cache-control"],
        "public, max-age=31536000, immutable"
    );
}

#[tokio::test]
async fn the_index_shell_should_not_get_an_immutable_cache_control() {
    let (repo_root, static_dir) = setup_fixtures();
    let router = router_with_static(repo_root.path(), static_dir.path());

    let (status, headers, _) = get_bytes_with_headers(router, "/").await;

    assert_eq!(status, StatusCode::OK);
    // Shells already carry their own validator-based `no-cache`
    // (docs/API.md); the point here is that the immutable-assets layer
    // didn't overwrite it.
    assert_eq!(headers["cache-control"], "no-cache");
}

#[tokio::test]
async fn a_missing_hashed_asset_should_404_without_an_immutable_cache_control() {
    let (repo_root, static_dir) = setup_fixtures();
    let router = router_with_static(repo_root.path(), static_dir.path());

    let (status, headers, body) = get_bytes_with_headers(router, "/_astro/missing.js").await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(headers["cache-control"], "no-cache");
    let body = String::from_utf8_lossy(&body);
    assert!(body.contains(NOT_FOUND_MARKER), "body was {body:?}");
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

// `router_for` sets no `AXGIT_STATIC_DIR`, so the two builds disagree on
// what an unmatched-but-shell-shaped path like `/nope` (one segment, the
// same shape `shell_for` gives `/{repo}`) should do: without `embed-web`,
// `Assets::resolve` finds neither a directory nor an embedded build, so no
// SPA fallback is installed at all and axum's own 404 applies; with
// `embed-web`, the binary's embedded `web/dist` copy serves as the fallback
// even with no directory configured (docs/DECISIONS.md #74) — the entire
// point of the feature — so the same request now serves the repo shell.
#[cfg(not(feature = "embed-web"))]
#[tokio::test]
async fn without_a_static_dir_unmatched_paths_should_still_404() {
    let repo_root = tempfile::tempdir().expect("failed to create repo root");
    let router = router_for(repo_root.path());

    let (status, _, _) = get_bytes_with_headers(router, "/nope").await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[cfg(feature = "embed-web")]
#[tokio::test]
async fn without_a_static_dir_the_embedded_build_should_serve_the_shell() {
    let repo_root = tempfile::tempdir().expect("failed to create repo root");
    let router = router_for(repo_root.path());

    let (status, headers, body) = get_bytes_with_headers(router, "/nope").await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        headers["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html"),
    );
    let body = String::from_utf8_lossy(&body);
    assert!(
        body.to_lowercase().contains("<html"),
        "expected an HTML shell body, got {body:?}"
    );
}
