//! git CLI based fixture helpers (docs/ARCHITECTURE.md test strategy).

// Shared across test binaries; not every binary uses every helper.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

/// Fixed date (RFC 3339) so commit-derived fields are deterministic.
pub const FIXED_DATE: &str = "2026-07-01T12:00:00+09:00";

/// Runs git isolated from host configuration, with fixed author/dates.
pub fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Test Author")
        .env("GIT_AUTHOR_EMAIL", "author@example.com")
        .env("GIT_COMMITTER_NAME", "Test Committer")
        .env("GIT_COMMITTER_EMAIL", "committer@example.com")
        .env("GIT_AUTHOR_DATE", FIXED_DATE)
        .env("GIT_COMMITTER_DATE", FIXED_DATE)
        .args(args)
        .output()
        .expect("failed to spawn git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
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

/// Creates branch `{name}` pointing at `main` in the bare repository.
pub fn add_branch(bare: &Path, name: &str) {
    git(bare, &["branch", name, "main"]);
}

/// Creates lightweight tag `{name}` on `main` in the bare repository.
pub fn add_lightweight_tag(bare: &Path, name: &str) {
    git(bare, &["tag", name, "main"]);
}

/// Creates annotated tag `{name}` on `main` in the bare repository.
pub fn add_annotated_tag(bare: &Path, name: &str, message: &str) {
    git(bare, &["tag", "-a", "-m", message, name, "main"]);
}

/// Sends `GET {uri}` to the router and returns status + parsed JSON body.
pub async fn get_json(router: Router, uri: &str) -> (StatusCode, Value) {
    let response = router
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&bytes).expect("response body is not JSON");
    (status, json)
}

/// Writes the cgit agefile (`info/web/last-modified`).
pub fn write_agefile(repo_dir: &Path, content: &str) {
    let dir = repo_dir.join("info/web");
    std::fs::create_dir_all(&dir).expect("failed to create info/web");
    std::fs::write(dir.join("last-modified"), content).expect("failed to write agefile");
}
