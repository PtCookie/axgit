//! Integration tests for the response cache and ETag/Cache-Control contract
//! (docs/ARCHITECTURE.md#caching, docs/API.md).
//!
//! Unlike the per-endpoint tests, these reuse one router across requests
//! (`Router` is cheaply clonable and shares the `AppState` response cache),
//! because the behavior under test *is* the shared cache.

mod common;

use axum::http::{StatusCode, header};

#[tokio::test]
async fn summary_should_serve_cached_body_until_validator_changes() {
    let root = tempfile::tempdir().unwrap();
    let bare = common::create_bare_repo(root.path(), "alpha.git");
    common::add_commit(&bare, "a.txt", "one\n", "first");
    common::set_meta(&bare, "cgit", "desc", "first description");
    let router = common::router_for(root.path());

    let (status, _, json) =
        common::get_json_with_headers(router.clone(), "/api/v1/repos/alpha").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["description"], "first description");

    // A config edit changes neither HEAD nor the agefile, so the cache keeps
    // serving the old body — the documented staleness window (TTL-bounded).
    common::set_meta(&bare, "cgit", "desc", "second description");
    let (status, _, json) =
        common::get_json_with_headers(router.clone(), "/api/v1/repos/alpha").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json["description"], "first description",
        "expected the cached body while the validator is unchanged"
    );

    // Touching the agefile (what the post-receive hook does) invalidates.
    common::write_agefile(&bare, "2026-07-24 13:06:00 +0900");
    let (status, _, json) = common::get_json_with_headers(router, "/api/v1/repos/alpha").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["description"], "second description");
}

#[tokio::test]
async fn commits_should_invalidate_on_head_move() {
    let root = tempfile::tempdir().unwrap();
    let bare = common::create_bare_repo(root.path(), "alpha.git");
    common::add_commit(&bare, "a.txt", "one\n", "first");
    let router = common::router_for(root.path());

    let (status, _, json) =
        common::get_json_with_headers(router.clone(), "/api/v1/repos/alpha/commits").await;
    assert_eq!(status, StatusCode::OK);
    let first_sha = json["commits"][0]["sha"].as_str().unwrap().to_owned();

    // A push moves HEAD; no agefile exists, so HEAD alone must invalidate.
    common::commit_history(
        &bare,
        &[common::CommitSpec {
            file: "a.txt",
            content: "two\n",
            message: "second",
            date: common::FIXED_DATE,
        }],
    );
    let (status, _, json) =
        common::get_json_with_headers(router, "/api/v1/repos/alpha/commits").await;
    assert_eq!(status, StatusCode::OK);
    let second_sha = json["commits"][0]["sha"].as_str().unwrap();
    assert_ne!(first_sha, second_sha);
    assert_eq!(json["commits"][0]["summary"], "second");
}

#[tokio::test]
async fn etag_roundtrip_should_return_304_until_invalidated() {
    let root = tempfile::tempdir().unwrap();
    let bare = common::create_bare_repo(root.path(), "alpha.git");
    common::add_commit(&bare, "a.txt", "one\n", "first");
    let router = common::router_for(root.path());

    let (status, headers, _) =
        common::get_json_with_headers(router.clone(), "/api/v1/repos/alpha").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
    let etag = headers[header::ETAG].to_str().unwrap().to_owned();

    let (status, headers, body) = common::get_bytes_with_request_headers(
        router.clone(),
        "/api/v1/repos/alpha",
        &[("if-none-match", &etag)],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert!(body.is_empty());
    // RFC 9110: the 304 repeats the ETag the 200 would have carried.
    assert_eq!(headers[header::ETAG].to_str().unwrap(), etag);
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");

    // Invalidation changes the ETag, so the conditional request gets a 200.
    common::write_agefile(&bare, "2026-07-24 13:06:00 +0900");
    let (status, headers, _) = common::get_json_with_request_headers(
        router,
        "/api/v1/repos/alpha",
        &[("if-none-match", &etag)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(headers[header::ETAG].to_str().unwrap(), etag);
}

#[tokio::test]
async fn sha_addressed_diff_should_hit_without_touching_repo() {
    let root = tempfile::tempdir().unwrap();
    let bare = common::create_bare_repo(root.path(), "alpha.git");
    common::add_commit(&bare, "a.txt", "one\n", "first");
    let router = common::router_for(root.path());

    let (_, _, json) =
        common::get_json_with_headers(router.clone(), "/api/v1/repos/alpha/commits").await;
    let sha = json["commits"][0]["sha"].as_str().unwrap().to_owned();
    let uri = format!("/api/v1/repos/alpha/commits/{sha}/diff");

    let (status, headers, json) = common::get_json_with_headers(router.clone(), &uri).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers[header::CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );
    let cached_files = json["files"].clone();

    // Immutable entries are served without opening the repository: deleting
    // it proves the hit path never touches the disk.
    std::fs::remove_dir_all(&bare).unwrap();
    let (status, _, json) = common::get_json_with_headers(router.clone(), &uri).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["files"], cached_files);

    // Validator-backed requests must notice the repository is gone.
    let (status, _) = common::get_json(router, "/api/v1/repos/alpha/commits/main/diff").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn cursor_addressed_commit_page_should_hit_without_touching_repo() {
    let root = tempfile::tempdir().unwrap();
    let bare = common::create_bare_repo(root.path(), "alpha.git");
    common::commit_history(
        &bare,
        &[
            common::CommitSpec {
                file: "a.txt",
                content: "one\n",
                message: "first",
                date: "2026-07-01T12:00:00+09:00",
            },
            common::CommitSpec {
                file: "a.txt",
                content: "two\n",
                message: "second",
                date: "2026-07-01T13:00:00+09:00",
            },
            common::CommitSpec {
                file: "a.txt",
                content: "three\n",
                message: "third",
                date: "2026-07-01T14:00:00+09:00",
            },
        ],
    );
    let router = common::router_for(root.path());

    let (_, _, json) =
        common::get_json_with_headers(router.clone(), "/api/v1/repos/alpha/commits?limit=1").await;
    let cursor = json["next_cursor"]
        .as_str()
        .expect("expected a next_cursor")
        .to_owned();
    let uri = format!("/api/v1/repos/alpha/commits?limit=1&cursor={cursor}");

    let (status, headers, json) = common::get_json_with_headers(router.clone(), &uri).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers[header::CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );
    let cached_commits = json["commits"].clone();

    // Immutable entries are served without opening the repository: deleting
    // it proves the hit path never touches the disk.
    std::fs::remove_dir_all(&bare).unwrap();
    let (status, _, json) = common::get_json_with_headers(router.clone(), &uri).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["commits"], cached_commits);

    // Validator-backed requests (no cursor) must still notice the repo is gone.
    let (status, _) = common::get_json(router, "/api/v1/repos/alpha/commits?limit=1").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn cursor_addressed_commit_page_should_survive_a_push() {
    let root = tempfile::tempdir().unwrap();
    let bare = common::create_bare_repo(root.path(), "alpha.git");
    common::commit_history(
        &bare,
        &[
            common::CommitSpec {
                file: "a.txt",
                content: "one\n",
                message: "first",
                date: "2026-07-01T12:00:00+09:00",
            },
            common::CommitSpec {
                file: "a.txt",
                content: "two\n",
                message: "second",
                date: "2026-07-01T13:00:00+09:00",
            },
            common::CommitSpec {
                file: "a.txt",
                content: "three\n",
                message: "third",
                date: "2026-07-01T14:00:00+09:00",
            },
        ],
    );
    let router = common::router_for(root.path());

    let (_, _, json) =
        common::get_json_with_headers(router.clone(), "/api/v1/repos/alpha/commits?limit=1").await;
    let cursor = json["next_cursor"]
        .as_str()
        .expect("expected a next_cursor")
        .to_owned();
    let uri = format!("/api/v1/repos/alpha/commits?limit=1&cursor={cursor}");

    let (_, _, before) = common::get_json_with_headers(router.clone(), &uri).await;

    // A push moves HEAD but leaves the cursor's pinned start commit
    // untouched: the page it addresses provably cannot change, so — unlike
    // every validator-backed entry — it survives the push instead of being
    // evicted.
    common::commit_history(
        &bare,
        &[common::CommitSpec {
            file: "a.txt",
            content: "four\n",
            message: "fourth",
            date: common::FIXED_DATE,
        }],
    );

    let (status, headers, after) = common::get_json_with_headers(router.clone(), &uri).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers[header::CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );
    assert_eq!(after["commits"], before["commits"]);

    // Page 1 (no cursor) is still validator-backed and reflects the push.
    let (_, _, page1) =
        common::get_json_with_headers(router, "/api/v1/repos/alpha/commits?limit=1").await;
    assert_eq!(page1["commits"][0]["summary"], "fourth");
}

#[tokio::test]
async fn archive_should_return_304_for_matching_weak_etag() {
    let root = tempfile::tempdir().unwrap();
    let bare = common::create_bare_repo(root.path(), "alpha.git");
    common::add_commit(&bare, "a.txt", "one\n", "first");
    let router = common::router_for(root.path());

    let (status, headers, _) =
        common::get_bytes_with_headers(router.clone(), "/api/v1/repos/alpha/archive/main.tar.gz")
            .await;
    assert_eq!(status, StatusCode::OK);
    let etag = headers[header::ETAG].to_str().unwrap().to_owned();
    assert!(etag.starts_with("W/\""), "expected weak etag, got {etag}");

    let (status, _, body) = common::get_bytes_with_request_headers(
        router,
        "/api/v1/repos/alpha/archive/main.tar.gz",
        &[("if-none-match", &etag)],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert!(body.is_empty());
}

#[tokio::test]
async fn list_repos_should_roundtrip_body_hash_etag() {
    let root = tempfile::tempdir().unwrap();
    let bare = common::create_bare_repo(root.path(), "alpha.git");
    common::add_commit(&bare, "a.txt", "one\n", "first");
    let router = common::router_for(root.path());

    let (status, headers, _) = common::get_json_with_headers(router.clone(), "/api/v1/repos").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
    let etag = headers[header::ETAG].to_str().unwrap().to_owned();

    let (status, _, body) = common::get_bytes_with_request_headers(
        router,
        "/api/v1/repos",
        &[("if-none-match", &etag)],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert!(body.is_empty());
}
