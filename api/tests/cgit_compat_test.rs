//! Integration tests for cgit-style URL compatibility redirects
//! (docs/DECISIONS.md #35), exercised against the full router so route
//! precedence (Smart HTTP, real static files) is covered, not just the
//! `cgit_compat::redirect_for` mapping itself (see `api/src/cgit_compat.rs`'s
//! own unit tests for the mapping's edge cases).

mod common;

use axum::http::StatusCode;

use common::{create_bare_repo, get_bytes_with_headers, router_with_static};

const INDEX_HTML: &str = "<!doctype html><html><body>INDEX</body></html>";
const REPO_HTML: &str = "<!doctype html><html><body>REPO SUMMARY</body></html>";
const NOT_FOUND_HTML: &str = "<!doctype html><html><body>NOT FOUND</body></html>";

/// Same shell fixture set as `static_shell_test.rs`, trimmed to what these
/// tests actually assert against (index, repo, 404).
fn setup_fixtures() -> (tempfile::TempDir, tempfile::TempDir) {
    let repo_root = tempfile::tempdir().expect("failed to create repo root");
    create_bare_repo(repo_root.path(), "axgit.git");

    let static_dir = tempfile::tempdir().expect("failed to create static dir");
    std::fs::write(static_dir.path().join("index.html"), INDEX_HTML).unwrap();
    std::fs::write(static_dir.path().join("404.html"), NOT_FOUND_HTML).unwrap();
    std::fs::create_dir_all(static_dir.path().join("__repo__")).unwrap();
    std::fs::write(static_dir.path().join("__repo__/index.html"), REPO_HTML).unwrap();

    (repo_root, static_dir)
}

async fn assert_redirect(router: axum::Router, uri: &str, expected_location: &str) {
    let (status, headers, _) = get_bytes_with_headers(router, uri).await;

    assert_eq!(status, StatusCode::PERMANENT_REDIRECT, "uri {uri}");
    assert_eq!(
        headers["location"], expected_location,
        "uri {uri} redirected to the wrong location"
    );
}

#[tokio::test]
async fn bare_dot_git_redirects_to_the_repo_page() {
    let (repo_root, static_dir) = setup_fixtures();
    let router = router_with_static(repo_root.path(), static_dir.path());

    assert_redirect(router, "/axgit.git", "/axgit").await;
}

#[tokio::test]
async fn commit_query_shape_redirects_to_the_commit_page() {
    let (repo_root, static_dir) = setup_fixtures();
    let router = router_with_static(repo_root.path(), static_dir.path());

    assert_redirect(
        router,
        "/axgit.git/commit/?id=abc123def456",
        "/axgit/commit/abc123def456",
    )
    .await;
}

#[tokio::test]
async fn log_with_h_redirects_to_the_log_page_with_ref_query() {
    let (repo_root, static_dir) = setup_fixtures();
    let router = router_with_static(repo_root.path(), static_dir.path());

    assert_redirect(router, "/axgit.git/log/?h=main", "/axgit/log?ref=main").await;
}

#[tokio::test]
async fn native_routes_are_not_redirected() {
    let (repo_root, static_dir) = setup_fixtures();
    let router = router_with_static(repo_root.path(), static_dir.path());

    let (status, _, body) = get_bytes_with_headers(router, "/axgit").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(String::from_utf8(body).unwrap(), REPO_HTML);
}

#[tokio::test]
async fn smart_http_routes_take_precedence_over_cgit_redirects() {
    let (repo_root, static_dir) = setup_fixtures();
    let router = router_with_static(repo_root.path(), static_dir.path());

    // `/axgit.git/info/refs` is also a `.git`-suffixed path `cgit_compat`
    // would otherwise redirect (to `/axgit/info/refs`), but the Smart HTTP
    // route is registered on the router itself, ahead of the fallback where
    // redirects run, so it must win and answer for real.
    let (status, headers, _) =
        get_bytes_with_headers(router, "/axgit.git/info/refs?service=git-upload-pack").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers["content-type"],
        "application/x-git-upload-pack-advertisement"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dot_git_clone_still_works() {
    let (repo_root, static_dir) = setup_fixtures();
    let router = router_with_static(repo_root.path(), static_dir.path());
    let addr = common::serve(router).await;

    let work = tempfile::tempdir().expect("failed to create work dir");
    common::git(
        work.path(),
        &["clone", "--quiet", &format!("http://{addr}/axgit.git"), "."],
    );
}
