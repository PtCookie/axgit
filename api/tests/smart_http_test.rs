//! Integration tests for Smart HTTP upload-pack: oneshot protocol checks and
//! real `git clone http://…` round-trips (docs/ARCHITECTURE.md test strategy).

mod common;

use std::io::Write;
use std::path::{Path, PathBuf};

use axum::http::{StatusCode, header};

use common::CommitSpec;
use common::router_for;

/// `clone.git` with two commits on `main`; returns (bare path, shas).
fn fixture(root: &Path) -> (PathBuf, Vec<String>) {
    let bare = common::create_bare_repo(root, "clone.git");
    let shas = common::commit_history(
        &bare,
        &[
            CommitSpec {
                file: "README.md",
                content: "# Clone fixture\n",
                message: "Initial commit",
                date: "2026-07-01T09:00:00+09:00",
            },
            CommitSpec {
                file: "src/main.rs",
                content: "fn main() {}\n",
                message: "Add main.rs",
                date: "2026-07-02T09:00:00+09:00",
            },
        ],
    );
    (bare, shas)
}

/// Builds a protocol v0 fetch request body: want + flush + done.
fn v0_fetch_body(sha: &str) -> Vec<u8> {
    format!("0032want {sha}\n00000009done\n").into_bytes()
}

#[tokio::test]
async fn info_refs_should_advertise_upload_pack() {
    let root = tempfile::tempdir().unwrap();
    let (_bare, shas) = fixture(root.path());

    let (status, headers, body) = common::get_bytes_with_headers(
        router_for(root.path()),
        "/clone.git/info/refs?service=git-upload-pack",
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers[header::CONTENT_TYPE],
        "application/x-git-upload-pack-advertisement"
    );
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
    assert!(body.starts_with(b"001e# service=git-upload-pack\n0000"));
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("refs/heads/main"), "advertisement: {text}");
    assert!(text.contains(shas.last().unwrap().as_str()));
}

#[tokio::test]
async fn info_refs_should_advertise_protocol_v2_when_requested() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());

    let (status, _, body) = common::get_bytes_with_request_headers(
        router_for(root.path()),
        "/clone.git/info/refs?service=git-upload-pack",
        &[("Git-Protocol", "version=2")],
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let after_preamble = &body[b"001e# service=git-upload-pack\n0000".len()..];
    assert!(
        after_preamble.starts_with(b"000eversion 2\n"),
        "v2 advertisement: {}",
        String::from_utf8_lossy(after_preamble)
    );
}

#[tokio::test]
async fn info_refs_should_reject_missing_or_unknown_service() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());

    for uri in ["/clone.git/info/refs", "/clone.git/info/refs?service=foo"] {
        let (status, json) = common::get_json(router_for(root.path()), uri).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "uri {uri}");
        assert_eq!(json["error"]["code"], "invalid_param");
    }
}

#[tokio::test]
async fn smart_http_should_404_unknown_repo() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());

    // Unknown repository and a path without the `.git` suffix.
    for uri in [
        "/missing.git/info/refs?service=git-upload-pack",
        "/clone/info/refs?service=git-upload-pack",
    ] {
        let (status, json) = common::get_json(router_for(root.path()), uri).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "uri {uri}");
        assert_eq!(json["error"]["code"], "repo_not_found");
    }

    let (status, _, _) = common::post_bytes_with_headers(
        router_for(root.path()),
        "/missing.git/git-upload-pack",
        &[],
        b"0000".to_vec(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn upload_pack_should_stream_pack() {
    let root = tempfile::tempdir().unwrap();
    let (_bare, shas) = fixture(root.path());
    let head = shas.last().unwrap();

    let (status, headers, body) = common::post_bytes_with_headers(
        router_for(root.path()),
        "/clone.git/git-upload-pack",
        &[],
        v0_fetch_body(head),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers[header::CONTENT_TYPE],
        "application/x-git-upload-pack-result"
    );
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
    assert!(body.starts_with(b"0008NAK\n"), "response start");
    assert!(
        body.windows(4).any(|window| window == b"PACK"),
        "no PACK signature in response"
    );
}

#[tokio::test]
async fn upload_pack_should_inflate_gzip_body() {
    let root = tempfile::tempdir().unwrap();
    let (_bare, shas) = fixture(root.path());

    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(&v0_fetch_body(shas.last().unwrap()))
        .unwrap();
    let gzipped = encoder.finish().unwrap();

    let (status, _, body) = common::post_bytes_with_headers(
        router_for(root.path()),
        "/clone.git/git-upload-pack",
        &[("Content-Encoding", "gzip")],
        gzipped,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.starts_with(b"0008NAK\n"));
    assert!(body.windows(4).any(|window| window == b"PACK"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clone_roundtrip() {
    let root = tempfile::tempdir().unwrap();
    let (_bare, shas) = fixture(root.path());
    let addr = common::serve(router_for(root.path())).await;

    let work = tempfile::tempdir().unwrap();
    common::git(
        work.path(),
        &["clone", "--quiet", &format!("http://{addr}/clone.git"), "."],
    );

    assert_eq!(
        common::git_output(work.path(), &["rev-parse", "HEAD"], &[]),
        *shas.last().unwrap()
    );
    let readme = std::fs::read_to_string(work.path().join("README.md")).unwrap();
    assert_eq!(readme, "# Clone fixture\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shallow_clone_roundtrip() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    let addr = common::serve(router_for(root.path())).await;

    let work = tempfile::tempdir().unwrap();
    common::git(
        work.path(),
        &[
            "clone",
            "--quiet",
            "--depth",
            "1",
            &format!("http://{addr}/clone.git"),
            ".",
        ],
    );

    assert_eq!(
        common::git_output(work.path(), &["rev-list", "--count", "HEAD"], &[]),
        "1"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetch_negotiation_roundtrip() {
    let root = tempfile::tempdir().unwrap();
    let (bare, _shas) = fixture(root.path());
    let addr = common::serve(router_for(root.path())).await;

    let work = tempfile::tempdir().unwrap();
    common::git(
        work.path(),
        &["clone", "--quiet", &format!("http://{addr}/clone.git"), "."],
    );

    // Extend the bare history after the clone so the fetch has haves to send.
    let new_shas = common::commit_history(
        &bare,
        &[CommitSpec {
            file: "CHANGELOG.md",
            content: "new commit\n",
            message: "Add changelog",
            date: "2026-07-03T09:00:00+09:00",
        }],
    );

    common::git(work.path(), &["fetch", "--quiet", "origin"]);
    assert_eq!(
        common::git_output(work.path(), &["rev-parse", "origin/main"], &[]),
        *new_shas.last().unwrap()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn push_should_be_rejected_over_http() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    let addr = common::serve(router_for(root.path())).await;

    let work = tempfile::tempdir().unwrap();
    common::git(
        work.path(),
        &["clone", "--quiet", &format!("http://{addr}/clone.git"), "."],
    );
    std::fs::write(work.path().join("evil.txt"), "push me\n").unwrap();
    common::commit_all(work.path(), "Attempt push over http");

    let stderr = common::git_expect_failure(work.path(), &["push", "origin", "main"]);
    assert!(
        stderr.contains("403"),
        "push was not rejected with 403: {stderr}"
    );
}
