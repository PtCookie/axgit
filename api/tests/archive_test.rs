//! Integration tests for the archive endpoint.

mod common;

use common::router_for;

use std::path::Path;
use std::process::Command;

use axum::http::{StatusCode, header};

const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// `archive.git` with README.md and src/main.rs on `main` plus branch
/// `feature/x`; returns (bare path, head sha).
fn setup_archive_repo(root: &Path) -> (std::path::PathBuf, String) {
    let bare = common::create_bare_repo(root, "archive.git");
    let shas = common::commit_history(
        &bare,
        &[
            common::CommitSpec {
                file: "README.md",
                content: "# Archive fixture\n",
                message: "add readme",
                date: "2026-07-01T12:00:00+09:00",
            },
            common::CommitSpec {
                file: "src/main.rs",
                content: "fn main() {}\n",
                message: "add main",
                date: "2026-07-02T12:00:00+09:00",
            },
        ],
    );
    common::add_branch(&bare, "feature/x");
    (bare, shas.last().unwrap().clone())
}

/// Extracts a tar.gz body with the system tar and returns the extraction dir.
fn extract_tar_gz(body: &[u8]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("failed to create extract dir");
    let archive = dir.path().join("archive.tar.gz");
    std::fs::write(&archive, body).expect("failed to write archive");
    let output = Command::new("tar")
        .current_dir(dir.path())
        .args(["-xzf", "archive.tar.gz"])
        .output()
        .expect("failed to spawn tar");
    assert!(
        output.status.success(),
        "tar failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    dir
}

#[tokio::test]
async fn archive_tar_gz_should_stream_extractable_archive() {
    let root = tempfile::tempdir().unwrap();
    let _ = setup_archive_repo(root.path());
    let (status, headers, body) = common::get_bytes_with_headers(
        router_for(root.path()),
        "/api/v1/repos/archive/archive/main.tar.gz",
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "application/gzip");
    assert_eq!(
        headers[header::CONTENT_DISPOSITION],
        "attachment; filename=\"archive-main.tar.gz\""
    );
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    // Branch-addressed: mutable, so a weak ETag + no-cache instead of immutable.
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
    let etag = headers[header::ETAG].to_str().unwrap();
    assert!(etag.starts_with("W/\""), "expected weak etag, got {etag}");

    let extracted = extract_tar_gz(&body);
    let prefix = extracted.path().join("archive-main");
    assert_eq!(
        std::fs::read_to_string(prefix.join("README.md")).unwrap(),
        "# Archive fixture\n"
    );
    assert_eq!(
        std::fs::read_to_string(prefix.join("src/main.rs")).unwrap(),
        "fn main() {}\n"
    );
}

#[tokio::test]
async fn archive_zip_should_match_git_archive_output() {
    let root = tempfile::tempdir().unwrap();
    let (bare, sha) = setup_archive_repo(root.path());
    let (status, headers, body) = common::get_bytes_with_headers(
        router_for(root.path()),
        "/api/v1/repos/archive/archive/main.zip",
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "application/zip");
    assert_eq!(
        headers[header::CONTENT_DISPOSITION],
        "attachment; filename=\"archive-main.zip\""
    );
    assert!(body.starts_with(b"PK\x03\x04"), "body is not a zip");

    // Same git binary + same arguments → byte-identical output.
    let expected = Command::new("git")
        .arg("--git-dir")
        .arg(&bare)
        .args([
            "archive",
            "--format",
            "zip",
            "--prefix",
            "archive-main/",
            &sha,
        ])
        .output()
        .expect("failed to spawn git archive");
    assert!(expected.status.success());
    assert_eq!(body, expected.stdout);
}

#[tokio::test]
async fn archive_by_full_sha_should_be_immutable() {
    let root = tempfile::tempdir().unwrap();
    let (_, sha) = setup_archive_repo(root.path());
    let (status, headers, body) = common::get_bytes_with_headers(
        router_for(root.path()),
        &format!("/api/v1/repos/archive/archive/{sha}.tar.gz"),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CACHE_CONTROL], IMMUTABLE);
    assert_eq!(
        headers[header::CONTENT_DISPOSITION],
        format!("attachment; filename=\"archive-{sha}.tar.gz\"")
    );

    // The archive root directory carries the sha as well.
    let extracted = extract_tar_gz(&body);
    assert!(
        extracted
            .path()
            .join(format!("archive-{sha}"))
            .join("README.md")
            .exists()
    );
}

#[tokio::test]
async fn archive_should_sanitize_slash_branch_names() {
    let root = tempfile::tempdir().unwrap();
    let _ = setup_archive_repo(root.path());
    let (status, headers, body) = common::get_bytes_with_headers(
        router_for(root.path()),
        "/api/v1/repos/archive/archive/feature/x.tar.gz",
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers[header::CONTENT_DISPOSITION],
        "attachment; filename=\"archive-feature-x.tar.gz\""
    );
    let extracted = extract_tar_gz(&body);
    assert!(
        extracted
            .path()
            .join("archive-feature-x/README.md")
            .exists()
    );
}

#[tokio::test]
async fn archive_should_return_404_for_missing_repo_and_ref() {
    let root = tempfile::tempdir().unwrap();
    let _ = setup_archive_repo(root.path());

    let (status, _, body) = common::get_bytes_with_headers(
        router_for(root.path()),
        "/api/v1/repos/missing/archive/main.tar.gz",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["code"], "repo_not_found");

    let (status, _, body) = common::get_bytes_with_headers(
        router_for(root.path()),
        "/api/v1/repos/archive/archive/no-such-ref.tar.gz",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["code"], "ref_not_found");
}

#[tokio::test]
async fn archive_should_return_400_for_unsupported_format() {
    let root = tempfile::tempdir().unwrap();
    let _ = setup_archive_repo(root.path());
    for rest in ["main.rar", "main.tar", "main", ".tar.gz"] {
        let (status, _, body) = common::get_bytes_with_headers(
            router_for(root.path()),
            &format!("/api/v1/repos/archive/archive/{rest}"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "rest {rest:?}");
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "invalid_param");
    }
}
