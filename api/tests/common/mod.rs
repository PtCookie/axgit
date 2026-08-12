//! git CLI based fixture helpers (docs/ARCHITECTURE.md test strategy).

// Shared across test binaries; not every binary uses every helper.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use axgit::config::Config;
use axgit::routes::build_router;
use axgit::state::AppState;

/// Fixed date (RFC 3339) so commit-derived fields are deterministic.
pub const FIXED_DATE: &str = "2026-07-01T12:00:00+09:00";

/// Test `Config` with every non-path field at its default; tests that need a
/// specific setting (e.g. `clone_url_base`) override the field afterwards.
pub fn test_config(repo_root: &Path) -> Config {
    Config {
        repo_root: repo_root.to_owned(),
        static_dir: None,
        listen: "127.0.0.1:0".parse().unwrap(),
        clone_url_base: None,
        cache_scan_ttl_secs: 60,
        cache_response_ttl_secs: 300,
        cache_response_max_bytes: 32 * 1024 * 1024,
    }
}

/// Router over `repo_root` with the default test configuration.
pub fn router_for(repo_root: &Path) -> Router {
    build_router(AppState::new(test_config(repo_root)))
}

/// Router with a static frontend build directory set, for SPA fallback tests.
pub fn router_with_static(repo_root: &Path, static_dir: &Path) -> Router {
    let mut config = test_config(repo_root);
    config.static_dir = Some(static_dir.to_owned());
    build_router(AppState::new(config))
}

/// Like [`router_with_static`], plus a `clone_url_base` — for asserting the
/// page shell's injected `rel="vcs-git"` link (docs/DECISIONS.md #63).
pub fn router_with_static_and_clone_base(
    repo_root: &Path,
    static_dir: &Path,
    clone_url_base: &str,
) -> Router {
    let mut config = test_config(repo_root);
    config.static_dir = Some(static_dir.to_owned());
    config.clone_url_base = Some(clone_url_base.to_owned());
    build_router(AppState::new(config))
}

/// Runs git isolated from host configuration, with fixed author/dates.
pub fn git(dir: &Path, args: &[&str]) {
    git_output(dir, args, &[]);
}

/// Like [`git`], but returns trimmed stdout and lets callers override the
/// fixed environment (e.g. per-commit `GIT_AUTHOR_DATE`).
pub fn git_output(dir: &Path, args: &[&str], extra_env: &[(&str, &str)]) -> String {
    let output = git_raw(dir, args, extra_env);
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// Runs git expecting failure and returns its stderr (e.g. a rejected push).
pub fn git_expect_failure(dir: &Path, args: &[&str]) -> String {
    let output = git_raw(dir, args, &[]);
    assert!(
        !output.status.success(),
        "git {args:?} unexpectedly succeeded: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn git_raw(dir: &Path, args: &[&str], extra_env: &[(&str, &str)]) -> std::process::Output {
    let mut command = Command::new("git");
    command
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Test Author")
        .env("GIT_AUTHOR_EMAIL", "author@example.com")
        .env("GIT_COMMITTER_NAME", "Test Committer")
        .env("GIT_COMMITTER_EMAIL", "committer@example.com")
        .env("GIT_AUTHOR_DATE", FIXED_DATE)
        .env("GIT_COMMITTER_DATE", FIXED_DATE);
    for (key, value) in extra_env {
        command.env(key, value);
    }
    command.args(args).output().expect("failed to spawn git")
}

/// Creates `{root}/{name}` as a bare repository with `main` as initial branch.
pub fn create_bare_repo(root: &Path, name: &str) -> PathBuf {
    let path = root.join(name);
    git(
        root,
        &[
            "init",
            "--quiet",
            "--bare",
            "--initial-branch=main",
            path.to_str().unwrap(),
        ],
    );
    path
}

/// Sets `{section}.{key}` in the bare repository's config file.
pub fn set_meta(repo_dir: &Path, section: &str, key: &str, value: &str) {
    git(
        repo_dir,
        &[
            "config",
            "--file",
            "config",
            &format!("{section}.{key}"),
            value,
        ],
    );
}

/// Adds one commit to the bare repository's `main` via a throwaway work repo.
pub fn add_commit(bare: &Path, file: &str, content: &str, message: &str) {
    let work = tempfile::tempdir().expect("failed to create work dir");
    let work_path = work.path();
    git(
        work_path,
        &["init", "--quiet", "--initial-branch=main", "."],
    );
    std::fs::write(work_path.join(file), content).expect("failed to write file");
    git(work_path, &["add", "."]);
    git(work_path, &["commit", "--quiet", "-m", message]);
    git(
        work_path,
        &["push", "--quiet", bare.to_str().unwrap(), "main:main"],
    );
}

/// One commit of [`commit_history`].
pub struct CommitSpec {
    pub file: &'static str,
    pub content: &'static str,
    pub message: &'static str,
    /// RFC 3339, used for both author and committer date.
    pub date: &'static str,
}

/// Builds a linear history on the bare repository's `main` from one work repo
/// (unlike [`add_commit`], which force-pushes a single fresh commit).
/// Returns commit shas oldest → newest.
pub fn commit_history(bare: &Path, commits: &[CommitSpec]) -> Vec<String> {
    let work = tempfile::tempdir().expect("failed to create work dir");
    let work_path = work.path();
    git(
        work_path,
        &["clone", "--quiet", bare.to_str().unwrap(), "."],
    );
    let mut shas = Vec::new();
    for spec in commits {
        let file = work_path.join(spec.file);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).expect("failed to create parent dirs");
        }
        std::fs::write(&file, spec.content).expect("failed to write file");
        git(work_path, &["add", "."]);
        git_output(
            work_path,
            &["commit", "--quiet", "-m", spec.message],
            &[
                ("GIT_AUTHOR_DATE", spec.date),
                ("GIT_COMMITTER_DATE", spec.date),
            ],
        );
        shas.push(git_output(work_path, &["rev-parse", "HEAD"], &[]));
    }
    git(work_path, &["push", "--quiet", "origin", "HEAD:main"]);
    shas
}

/// Stages everything in the work repo and commits it; returns the new sha.
/// For histories the [`CommitSpec`] helpers cannot express (multi-file,
/// deletions, renames, binaries) — pair with a `git clone` of the bare repo.
pub fn commit_all(work_path: &Path, message: &str) -> String {
    git(work_path, &["add", "-A"]);
    git(work_path, &["commit", "--quiet", "-m", message]);
    git_output(work_path, &["rev-parse", "HEAD"], &[])
}

/// Creates branch `{name}` pointing at `main` in the bare repository.
pub fn add_branch(bare: &Path, name: &str) {
    git(bare, &["branch", name, "main"]);
}

/// Creates `refs/remotes/{name}` pointing at `target` (a treeish, e.g.
/// `"main"`) directly via `update-ref` — no real `git remote add`/`fetch`
/// involved, which is enough to exercise the remote-branch listing without
/// standing up an actual second repository to fetch from.
pub fn add_remote_branch(bare: &Path, name: &str, target: &str) {
    git(
        bare,
        &["update-ref", &format!("refs/remotes/{name}"), target],
    );
}

/// Creates the symbolic `refs/remotes/{remote}/HEAD` ref pointing at
/// `refs/remotes/{remote}/{branch}` — the alias `list_refs` must skip so it
/// doesn't show up as its own remote-branch row.
pub fn add_remote_head(bare: &Path, remote: &str, branch: &str) {
    git(
        bare,
        &[
            "symbolic-ref",
            &format!("refs/remotes/{remote}/HEAD"),
            &format!("refs/remotes/{remote}/{branch}"),
        ],
    );
}

/// Creates lightweight tag `{name}` on `main` in the bare repository.
pub fn add_lightweight_tag(bare: &Path, name: &str) {
    git(bare, &["tag", name, "main"]);
}

/// Creates annotated tag `{name}` on `main` in the bare repository.
pub fn add_annotated_tag(bare: &Path, name: &str, message: &str) {
    git(bare, &["tag", "-a", "-m", message, name, "main"]);
}

/// Creates annotated tag `{name}` on an arbitrary object (a blob, tree, or
/// another tag's oid) — [`add_annotated_tag`] only ever tags `main`.
pub fn add_annotated_tag_on(bare: &Path, name: &str, message: &str, object: &str) {
    git(bare, &["tag", "-a", "-m", message, name, object]);
}

/// Attaches a `git notes` message to `sha` on the repository's default notes
/// ref (`refs/notes/commits`). Works directly against a bare repository.
pub fn add_note(bare: &Path, sha: &str, message: &str) {
    git(bare, &["notes", "add", "-m", message, sha]);
}

/// Sends `GET {uri}` to the router and returns status + parsed JSON body.
pub async fn get_json(router: Router, uri: &str) -> (StatusCode, Value) {
    let (status, _, json) = get_json_with_headers(router, uri).await;
    (status, json)
}

/// Like [`get_json`], but also returns response headers (e.g. `Cache-Control`).
pub async fn get_json_with_headers(router: Router, uri: &str) -> (StatusCode, HeaderMap, Value) {
    let response = router
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&bytes).expect("response body is not JSON");
    (status, headers, json)
}

/// Like [`get_json_with_headers`], but attaches request headers first
/// (e.g. `If-None-Match`). Panics on an empty body — use
/// [`get_bytes_with_request_headers`] for expected 304s.
pub async fn get_json_with_request_headers(
    router: Router,
    uri: &str,
    request_headers: &[(&str, &str)],
) -> (StatusCode, HeaderMap, Value) {
    let (status, headers, bytes) =
        get_bytes_with_request_headers(router, uri, request_headers).await;
    let json = serde_json::from_slice(&bytes).expect("response body is not JSON");
    (status, headers, json)
}

/// Sends `GET {uri}` and returns status + headers + raw body bytes
/// (for non-JSON responses like the raw endpoint).
pub async fn get_bytes_with_headers(router: Router, uri: &str) -> (StatusCode, HeaderMap, Vec<u8>) {
    let response = router
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, bytes.to_vec())
}

/// Like [`get_bytes_with_headers`], but attaches request headers first
/// (e.g. `Host` / `X-Forwarded-*` for the feed's base-URL reconstruction).
pub async fn get_bytes_with_request_headers(
    router: Router,
    uri: &str,
    request_headers: &[(&str, &str)],
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut request = Request::get(uri);
    for (name, value) in request_headers {
        request = request.header(*name, *value);
    }
    let response = router
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, bytes.to_vec())
}

/// Sends `POST {uri}` with the given request headers and raw body
/// (for Smart HTTP pkt-line payloads).
pub async fn post_bytes_with_headers(
    router: Router,
    uri: &str,
    request_headers: &[(&str, &str)],
    body: Vec<u8>,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut request = Request::post(uri);
    for (name, value) in request_headers {
        request = request.header(*name, *value);
    }
    let response = router
        .oneshot(request.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, bytes.to_vec())
}

/// Serves the router on an ephemeral local port for tests that need a real
/// listener (`git clone http://…` round-trips). The server task is aborted
/// when the runtime shuts down at the end of the test.
pub async fn serve(router: Router) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind test listener");
    let addr = listener.local_addr().expect("listener has no local addr");
    tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("test server failed");
    });
    addr
}

/// Writes the cgit agefile (`info/web/last-modified`).
pub fn write_agefile(repo_dir: &Path, content: &str) {
    let dir = repo_dir.join("info/web");
    std::fs::create_dir_all(&dir).expect("failed to create info/web");
    std::fs::write(dir.join("last-modified"), content).expect("failed to write agefile");
}
