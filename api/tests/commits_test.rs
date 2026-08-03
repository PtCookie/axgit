//! Integration tests for `GET /api/v1/repos/{repo}/commits`.

mod common;

use std::path::Path;

use axum::http::StatusCode;
use serde_json::Value;
use tempfile::TempDir;

use common::CommitSpec;
use common::router_for;

/// sha256 of `author@example.com` (the fixed fixture author email).
const AUTHOR_EMAIL_HASH: &str = "b0eda69977c26118feff17875d53376006568bcbcde5ca0c916d01f05c281436";

/// `alpha.git` with a 5-commit linear history on `main` (shas oldest → newest):
/// a.txt, docs/guide.md, a.txt, b.txt, a.txt — distinct dates for ordering.
fn setup_history() -> (TempDir, Vec<String>) {
    let root = tempfile::tempdir().expect("failed to create fixture root");
    let alpha = common::create_bare_repo(root.path(), "alpha.git");
    let shas = common::commit_history(
        &alpha,
        &[
            CommitSpec {
                file: "a.txt",
                content: "one",
                message: "feat: add a",
                date: "2026-07-01T12:00:00+09:00",
            },
            CommitSpec {
                file: "docs/guide.md",
                content: "guide",
                message: "docs: add guide\n\nwith a body",
                date: "2026-07-01T13:00:00+09:00",
            },
            CommitSpec {
                file: "a.txt",
                content: "two",
                message: "fix: update a",
                date: "2026-07-01T14:00:00+09:00",
            },
            CommitSpec {
                file: "b.txt",
                content: "b",
                message: "feat: add b",
                date: "2026-07-01T15:00:00+09:00",
            },
            CommitSpec {
                file: "a.txt",
                content: "three",
                message: "fix: update a again",
                date: "2026-07-01T16:00:00+09:00",
            },
        ],
    );
    (root, shas)
}

async fn get_ok(root: &Path, uri: &str) -> Value {
    let (status, json) = common::get_json(router_for(root), uri).await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");
    json
}

fn shas_of(json: &Value) -> Vec<String> {
    json["commits"]
        .as_array()
        .expect("commits is not an array")
        .iter()
        .map(|commit| commit["sha"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn commits_returns_full_log_newest_first() {
    let (root, shas) = setup_history();

    let json = get_ok(root.path(), "/api/v1/repos/alpha/commits").await;

    let mut expected = shas.clone();
    expected.reverse();
    assert_eq!(shas_of(&json), expected, "unexpected response: {json}");
    assert_eq!(json["next_cursor"], Value::Null);

    let commits = json["commits"].as_array().unwrap();
    let newest = &commits[0];
    assert_eq!(
        (
            newest["summary"].as_str(),
            newest["authored_at"].as_str(),
            newest["parents"].as_array().map(Vec::len),
        ),
        (
            Some("fix: update a again"),
            Some("2026-07-01T16:00:00+09:00"),
            Some(1),
        ),
        "unexpected response: {json}"
    );
    // Multi-line message: summary is the first line only.
    assert_eq!(commits[3]["summary"], "docs: add guide");
    // Root commit has no parents; the others link to their predecessor.
    assert_eq!(commits[4]["parents"], Value::Array(vec![]));
    assert_eq!(commits[3]["parents"][0], Value::String(shas[0].clone()));
}

#[tokio::test]
async fn commits_paginates_without_overlap_or_gap() {
    let (root, shas) = setup_history();

    let mut collected = Vec::new();
    let mut uri = "/api/v1/repos/alpha/commits?limit=2".to_owned();
    let mut pages = Vec::new();
    loop {
        let json = get_ok(root.path(), &uri).await;
        pages.push(shas_of(&json).len());
        collected.extend(shas_of(&json));
        match json["next_cursor"].as_str() {
            Some(cursor) => uri = format!("/api/v1/repos/alpha/commits?limit=2&cursor={cursor}"),
            None => break,
        }
    }

    assert_eq!(pages, [2, 2, 1]);
    let mut expected = shas.clone();
    expected.reverse();
    assert_eq!(collected, expected);
}

#[tokio::test]
async fn commits_rejects_invalid_limit() {
    let (root, _shas) = setup_history();

    for limit in ["0", "101", "abc", "-1"] {
        let uri = format!("/api/v1/repos/alpha/commits?limit={limit}");
        let (status, json) = common::get_json(router_for(root.path()), &uri).await;
        assert_eq!(
            (status, json["error"]["code"].as_str()),
            (StatusCode::BAD_REQUEST, Some("invalid_param")),
            "limit={limit} was not rejected: {json}"
        );
    }

    let json = get_ok(root.path(), "/api/v1/repos/alpha/commits?limit=100").await;
    assert_eq!(shas_of(&json).len(), 5);
}

#[tokio::test]
async fn commits_resolves_ref_variants() {
    let (root, shas) = setup_history();
    let alpha = root.path().join("alpha.git");
    common::add_branch(&alpha, "dev");
    common::add_lightweight_tag(&alpha, "snapshot");
    common::add_annotated_tag(&alpha, "v1.0.0", "release v1.0.0");

    // Branch and tags point at the main tip.
    for refname in ["dev", "snapshot", "v1.0.0"] {
        let json = get_ok(
            root.path(),
            &format!("/api/v1/repos/alpha/commits?ref={refname}"),
        )
        .await;
        assert_eq!(shas_of(&json).len(), 5, "ref={refname}: {json}");
        assert_eq!(shas_of(&json)[0], shas[4], "ref={refname}: {json}");
    }

    // A commit sha roots the log at that commit.
    let json = get_ok(
        root.path(),
        &format!("/api/v1/repos/alpha/commits?ref={}", shas[2]),
    )
    .await;
    assert_eq!(
        shas_of(&json),
        [shas[2].as_str(), shas[1].as_str(), shas[0].as_str()]
    );
}

#[tokio::test]
async fn commits_returns_404_for_unknown_ref() {
    let (root, _shas) = setup_history();

    let (status, json) = common::get_json(
        router_for(root.path()),
        "/api/v1/repos/alpha/commits?ref=nope",
    )
    .await;

    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("ref_not_found")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn commits_rejects_invalid_cursor() {
    let (root, shas) = setup_history();

    for cursor in [
        "zzz",
        // The old bare-sha cursor format is no longer accepted.
        "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
        shas[4].as_str(),
        &format!("{}.abc", shas[4]),
        &format!("{}.-1", shas[4]),
        // Beyond Cursor::MAX_OFFSET (100_000).
        &format!("{}.100001", shas[4]),
        // Well-formed but pointing at a commit that doesn't exist.
        "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef.0",
    ] {
        let uri = format!("/api/v1/repos/alpha/commits?cursor={cursor}");
        let (status, json) = common::get_json(router_for(root.path()), &uri).await;
        assert_eq!(
            (status, json["error"]["code"].as_str()),
            (StatusCode::BAD_REQUEST, Some("invalid_param")),
            "cursor={cursor} was not rejected: {json}"
        );
    }
}

#[tokio::test]
async fn commits_filters_by_path() {
    let (root, shas) = setup_history();

    // File path: only the commits touching a.txt (indexes 0, 2, 4).
    let json = get_ok(root.path(), "/api/v1/repos/alpha/commits?path=a.txt").await;
    assert_eq!(
        shas_of(&json),
        [shas[4].as_str(), shas[2].as_str(), shas[0].as_str()]
    );

    // Directory path (with and without trailing slash) and nested file.
    for path in ["docs", "docs/", "docs/guide.md"] {
        let json = get_ok(
            root.path(),
            &format!("/api/v1/repos/alpha/commits?path={path}"),
        )
        .await;
        assert_eq!(shas_of(&json), [shas[1].as_str()], "path={path}: {json}");
    }

    // Unknown path: empty list, not 404.
    let json = get_ok(root.path(), "/api/v1/repos/alpha/commits?path=missing.txt").await;
    assert_eq!(shas_of(&json), Vec::<String>::new());
    assert_eq!(json["next_cursor"], Value::Null);
}

#[tokio::test]
async fn commits_paginates_within_path_filter() {
    let (root, shas) = setup_history();

    let json = get_ok(
        root.path(),
        "/api/v1/repos/alpha/commits?path=a.txt&limit=1",
    )
    .await;
    assert_eq!(shas_of(&json), [shas[4].as_str()]);
    // The cursor is now `<start-sha>.<offset>`, not the boundary commit's sha
    // (docs/DECISIONS.md #37) — assert a cursor is present, not its literal value.
    let cursor = json["next_cursor"]
        .as_str()
        .expect("expected a next_cursor")
        .to_owned();

    let uri = format!("/api/v1/repos/alpha/commits?path=a.txt&limit=1&cursor={cursor}");
    let json = get_ok(root.path(), &uri).await;
    assert_eq!(shas_of(&json), [shas[2].as_str()]);
    assert!(
        json["next_cursor"].is_string(),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn commits_exposes_email_hash_but_not_email() {
    let (root, _shas) = setup_history();

    let json = get_ok(root.path(), "/api/v1/repos/alpha/commits?limit=1").await;

    let author = &json["commits"][0]["author"];
    assert_eq!(
        (author["name"].as_str(), author["email_hash"].as_str()),
        (Some("Test Author"), Some(AUTHOR_EMAIL_HASH)),
        "unexpected response: {json}"
    );
    assert!(
        author.get("email").is_none(),
        "raw email must not be exposed: {json}"
    );
}

#[tokio::test]
async fn commits_returns_empty_page_for_empty_repo() {
    let root = tempfile::tempdir().expect("failed to create fixture root");
    common::create_bare_repo(root.path(), "empty.git");

    let json = get_ok(root.path(), "/api/v1/repos/empty/commits").await;
    assert_eq!(json["commits"], Value::Array(vec![]));
    assert_eq!(json["next_cursor"], Value::Null);

    // An explicit ref on an empty repo is a resolution failure, not an empty page.
    let (status, json) = common::get_json(
        router_for(root.path()),
        "/api/v1/repos/empty/commits?ref=main",
    )
    .await;
    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("ref_not_found")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn commits_returns_404_for_missing_repo() {
    let root = tempfile::tempdir().expect("failed to create fixture root");

    let (status, json) =
        common::get_json(router_for(root.path()), "/api/v1/repos/missing/commits").await;

    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("repo_not_found")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn commits_lists_both_parents_of_a_merge() {
    let root = tempfile::tempdir().expect("failed to create fixture root");
    let bare = common::create_bare_repo(root.path(), "merged.git");
    common::commit_history(
        &bare,
        &[CommitSpec {
            file: "base.txt",
            content: "base",
            message: "feat: base",
            date: "2026-07-01T12:00:00+09:00",
        }],
    );

    // Build a merge with raw git: main and feature diverge, then --no-ff merge.
    let work = tempfile::tempdir().expect("failed to create work dir");
    let work_path = work.path();
    common::git(
        work_path,
        &["clone", "--quiet", bare.to_str().unwrap(), "."],
    );
    common::git(work_path, &["checkout", "--quiet", "-b", "feature"]);
    std::fs::write(work_path.join("f.txt"), "f").unwrap();
    common::git(work_path, &["add", "."]);
    common::git(work_path, &["commit", "--quiet", "-m", "feat: on feature"]);
    common::git(work_path, &["checkout", "--quiet", "main"]);
    std::fs::write(work_path.join("m.txt"), "m").unwrap();
    common::git(work_path, &["add", "."]);
    common::git(work_path, &["commit", "--quiet", "-m", "feat: on main"]);
    common::git(
        work_path,
        &[
            "merge",
            "--no-ff",
            "--quiet",
            "-m",
            "merge feature",
            "feature",
        ],
    );
    common::git(work_path, &["push", "--quiet", "origin", "HEAD:main"]);

    let json = get_ok(root.path(), "/api/v1/repos/merged/commits?limit=1").await;

    let merge = &json["commits"][0];
    assert_eq!(
        (
            merge["summary"].as_str(),
            merge["parents"].as_array().map(Vec::len),
        ),
        (Some("merge feature"), Some(2)),
        "unexpected response: {json}"
    );
}

/// `branchy.git`: a DAG built to test that pagination doesn't drop side-branch
/// commits pending at a page boundary (docs/DECISIONS.md #37):
///
/// ```text
/// A(root) --- B --- C           (main)
///   \                 \
///    S1 ------- S2 -----M       (M's parents: C, S2)
/// ```
///
/// Dates are fixed (not creation order) so committer-date order — the walk's
/// order, `commits.rs::log` sets no sort flags — visits `M, C, S2, B, S1, A`.
/// Returns shas in that walk order.
fn setup_branching_history() -> (TempDir, Vec<String>) {
    let root = tempfile::tempdir().expect("failed to create fixture root");
    let bare = common::create_bare_repo(root.path(), "branchy.git");
    let work = tempfile::tempdir().expect("failed to create work dir");
    let work_path = work.path();
    common::git(
        work_path,
        &["clone", "--quiet", bare.to_str().unwrap(), "."],
    );

    let commit = |file: &str, message: &str, date: &str| -> String {
        std::fs::write(work_path.join(file), file).unwrap();
        common::git(work_path, &["add", "."]);
        common::git_output(
            work_path,
            &["commit", "--quiet", "-m", message],
            &[("GIT_AUTHOR_DATE", date), ("GIT_COMMITTER_DATE", date)],
        );
        common::git_output(work_path, &["rev-parse", "HEAD"], &[])
    };

    let a = commit("a.txt", "feat: a (root)", "2026-07-01T10:00:00+09:00");
    common::git(work_path, &["checkout", "--quiet", "-b", "side"]);
    let s1 = commit("s1.txt", "feat: s1", "2026-07-01T11:00:00+09:00");
    let s2 = commit("s2.txt", "feat: s2", "2026-07-01T13:00:00+09:00");
    common::git(work_path, &["checkout", "--quiet", "main"]);
    let b = commit("b.txt", "feat: b", "2026-07-01T12:00:00+09:00");
    let c = commit("c.txt", "feat: c", "2026-07-01T14:00:00+09:00");
    common::git_output(
        work_path,
        &["merge", "--no-ff", "--quiet", "-m", "merge side", "side"],
        &[
            ("GIT_AUTHOR_DATE", "2026-07-01T15:00:00+09:00"),
            ("GIT_COMMITTER_DATE", "2026-07-01T15:00:00+09:00"),
        ],
    );
    let m = common::git_output(work_path, &["rev-parse", "HEAD"], &[]);
    common::git(work_path, &["push", "--quiet", "origin", "HEAD:main"]);

    (root, vec![m, c, s2, b, s1, a])
}

/// Reproduces the bug DECISIONS.md #37 fixes: with the old single-sha
/// cursor, `S2`'s sibling pending commit `B` (not an ancestor of `S2`) was
/// silently dropped once page 2 pushed only `S2`. The offset cursor re-walks
/// from a fixed start every page, so this must match a single unpaginated
/// walk exactly.
#[tokio::test]
async fn commits_pagination_keeps_side_branch_commits() {
    let (root, expected) = setup_branching_history();

    let mut collected = Vec::new();
    let mut uri = "/api/v1/repos/branchy/commits?limit=2".to_owned();
    loop {
        let json = get_ok(root.path(), &uri).await;
        collected.extend(shas_of(&json));
        match json["next_cursor"].as_str() {
            Some(cursor) => uri = format!("/api/v1/repos/branchy/commits?limit=2&cursor={cursor}"),
            None => break,
        }
    }

    assert_eq!(
        collected, expected,
        "pagination must match a single unpaginated walk exactly, with no drops or duplicates"
    );
}
