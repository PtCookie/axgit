//! Integration tests for `GET /api/v1/repos/{repo}/rawdiff` and
//! `GET /api/v1/repos/{repo}/patch`.

mod common;

use std::path::Path;

use axum::http::{HeaderMap, StatusCode};
use tempfile::TempDir;

use common::commit_all;
use common::router_for;

/// `alpha.git`, `main` branch; shas oldest → newest:
/// 0. root: `a.txt` (2 lines)
/// 1. modify `a.txt`
/// 2. add `b.txt`
/// 3. modify `b.txt`
fn setup() -> (TempDir, Vec<String>) {
    let root = tempfile::tempdir().expect("failed to create fixture root");
    let bare = common::create_bare_repo(root.path(), "alpha.git");
    let work = tempfile::tempdir().expect("failed to create work dir");
    let work_path = work.path();
    common::git(
        work_path,
        &["clone", "--quiet", bare.to_str().unwrap(), "."],
    );
    let mut shas = Vec::new();

    std::fs::write(work_path.join("a.txt"), "one\ntwo\n").unwrap();
    shas.push(commit_all(work_path, "feat: initial file"));

    std::fs::write(work_path.join("a.txt"), "one\nthree\n").unwrap();
    shas.push(commit_all(work_path, "fix: update a"));

    std::fs::write(work_path.join("b.txt"), "bee\n").unwrap();
    shas.push(commit_all(work_path, "feat: add b"));

    std::fs::write(work_path.join("b.txt"), "bee\nsting\n").unwrap();
    shas.push(commit_all(work_path, "fix: update b"));

    common::git(work_path, &["push", "--quiet", "origin", "HEAD:main"]);
    (root, shas)
}

async fn get_bytes(repo_root: &Path, uri: &str) -> (StatusCode, HeaderMap, Vec<u8>) {
    common::get_bytes_with_headers(router_for(repo_root), uri).await
}

/// Clones `alpha.git` into a fresh work tree, checked out at `sha`.
fn clone_at(bare_path: &Path, sha: &str) -> TempDir {
    let work = tempfile::tempdir().unwrap();
    common::git(
        work.path(),
        &["clone", "--quiet", bare_path.to_str().unwrap(), "."],
    );
    common::git(work.path(), &["checkout", "--quiet", sha]);
    work
}

#[tokio::test]
async fn patch_should_apply_with_git_am() {
    let (root, shas) = setup();
    let (status, _headers, body) = get_bytes(
        root.path(),
        &format!("/api/v1/repos/alpha/patch?from={}&to={}", shas[0], shas[3]),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let patch_dir = tempfile::tempdir().unwrap();
    let patch_path = patch_dir.path().join("series.patch");
    std::fs::write(&patch_path, &body).unwrap();

    let bare_path = root.path().join("alpha.git");
    let apply_dir = clone_at(&bare_path, &shas[0]);
    common::git(
        apply_dir.path(),
        &["am", "--quiet", patch_path.to_str().unwrap()],
    );

    // Committer identity/date differ from the original (git am re-commits),
    // so compare trees rather than commit shas.
    let applied_tree = common::git_output(apply_dir.path(), &["rev-parse", "HEAD^{tree}"], &[]);
    let expected_tree = common::git_output(
        apply_dir.path(),
        &["rev-parse", &format!("{}^{{tree}}", shas[3])],
        &[],
    );
    assert_eq!(applied_tree, expected_tree);
}

#[tokio::test]
async fn rawdiff_should_apply_with_git_apply() {
    let (root, shas) = setup();
    let (status, _headers, body) = get_bytes(
        root.path(),
        &format!(
            "/api/v1/repos/alpha/rawdiff?from={}&to={}",
            shas[0], shas[1]
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let text = String::from_utf8(body).expect("rawdiff body was not utf-8");

    // Structural assertions rather than a byte comparison against `git diff`
    // output: libgit2 and git disagree on blob-hash abbreviation length and
    // on rename-detection defaults, so a byte-for-byte comparison would be
    // flaky. What matters is that it's a well-formed, applicable patch.
    assert!(
        text.contains("diff --git a/a.txt b/a.txt"),
        "missing file header: {text}"
    );
    assert!(
        text.contains("@@ -1,2 +1,2 @@"),
        "missing hunk header: {text}"
    );
    assert!(text.contains("-two"), "missing deletion line: {text}");
    assert!(text.contains("+three"), "missing addition line: {text}");

    let patch_dir = tempfile::tempdir().unwrap();
    let patch_path = patch_dir.path().join("diff.patch");
    std::fs::write(&patch_path, &text).unwrap();

    let bare_path = root.path().join("alpha.git");
    let apply_dir = clone_at(&bare_path, &shas[0]);
    common::git(
        apply_dir.path(),
        &["apply", "--check", patch_path.to_str().unwrap()],
    );
}

#[tokio::test]
async fn raw_output_should_not_apply_the_json_caps() {
    let root = tempfile::tempdir().unwrap();
    let bare = common::create_bare_repo(root.path(), "alpha.git");
    let work = tempfile::tempdir().unwrap();
    let work_path = work.path();
    common::git(
        work_path,
        &["clone", "--quiet", bare.to_str().unwrap(), "."],
    );
    let big: String = (0..1100).map(|i| format!("line {i}\n")).collect();
    std::fs::write(work_path.join("big.txt"), big).unwrap();
    let sha = commit_all(work_path, "feat: add big file");
    common::git(work_path, &["push", "--quiet", "origin", "HEAD:main"]);

    let (status, json) = common::get_json(
        router_for(root.path()),
        &format!("/api/v1/repos/alpha/commits/{sha}/diff"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let file = &json["files"][0];
    assert_eq!(
        file["truncated"].as_bool(),
        Some(true),
        "unexpected response: {json}"
    );

    let (status, _headers, body) = get_bytes(
        root.path(),
        &format!("/api/v1/repos/alpha/rawdiff?to={sha}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let text = String::from_utf8(body).unwrap();
    let additions = text.lines().filter(|line| line.starts_with('+')).count();
    // 1100 `+line N` lines, minus the `+++ b/big.txt` file-header line.
    assert_eq!(
        additions, 1101,
        "rawdiff should render every added line, not just the first 1000: {text}"
    );
}

#[tokio::test]
async fn rawdiff_should_set_content_type_and_nosniff() {
    let (root, shas) = setup();
    let (status, headers, _body) = get_bytes(
        root.path(),
        &format!(
            "/api/v1/repos/alpha/rawdiff?from={}&to={}",
            shas[0], shas[1]
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get("content-type").map(|v| v.to_str().unwrap()),
        Some("text/plain; charset=utf-8")
    );
    assert_eq!(
        headers
            .get("x-content-type-options")
            .map(|v| v.to_str().unwrap()),
        Some("nosniff")
    );
}

#[tokio::test]
async fn patch_should_set_content_disposition_and_robots_headers() {
    let (root, shas) = setup();
    let (status, headers, _body) = get_bytes(
        root.path(),
        &format!("/api/v1/repos/alpha/patch?to={}", shas[0]),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get("content-type").map(|v| v.to_str().unwrap()),
        Some("text/plain; charset=utf-8")
    );
    assert_eq!(
        headers
            .get("x-content-type-options")
            .map(|v| v.to_str().unwrap()),
        Some("nosniff")
    );
    assert_eq!(
        headers.get("x-robots-tag").map(|v| v.to_str().unwrap()),
        Some("noindex, nofollow")
    );
    let disposition = headers
        .get("content-disposition")
        .map(|v| v.to_str().unwrap())
        .unwrap_or_default();
    assert!(
        disposition.starts_with("inline; filename=\"alpha-"),
        "unexpected content-disposition: {disposition}"
    );
    assert!(
        disposition.ends_with(".patch\""),
        "unexpected content-disposition: {disposition}"
    );
}

#[tokio::test]
async fn patch_should_emit_one_block_per_commit_in_a_range() {
    let (root, shas) = setup();
    let (status, _headers, body) = get_bytes(
        root.path(),
        &format!("/api/v1/repos/alpha/patch?from={}&to={}", shas[0], shas[3]),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let text = String::from_utf8(body).unwrap();

    let block_count = text.matches("Mon Sep 17 00:00:00 2001").count();
    assert_eq!(
        block_count, 3,
        "expected one From <sha> Mon Sep 17 ... block per commit in (from, to]: {text}"
    );
    assert!(text.contains("[PATCH 1/3]"), "{text}");
    assert!(text.contains("[PATCH 2/3]"), "{text}");
    assert!(text.contains("[PATCH 3/3]"), "{text}");
    // Oldest-to-newest order.
    let update_a_pos = text.find("fix: update a").unwrap();
    let add_b_pos = text.find("feat: add b").unwrap();
    let update_b_pos = text.find("fix: update b").unwrap();
    assert!(
        update_a_pos < add_b_pos && add_b_pos < update_b_pos,
        "{text}"
    );
}

#[tokio::test]
async fn patch_should_default_to_a_single_commit() {
    let (root, shas) = setup();
    let (status, _headers, body) = get_bytes(
        root.path(),
        &format!("/api/v1/repos/alpha/patch?to={}", shas[1]),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let text = String::from_utf8(body).unwrap();
    let block_count = text.matches("Mon Sep 17 00:00:00 2001").count();
    assert_eq!(block_count, 1, "{text}");
    assert!(text.contains("fix: update a"), "{text}");
    // A single-patch series omits the n/m counter entirely.
    assert!(!text.contains("[PATCH 1/1]"), "{text}");
}

#[tokio::test]
async fn patch_should_reject_a_range_over_the_commit_cap() {
    let root = tempfile::tempdir().unwrap();
    let bare = common::create_bare_repo(root.path(), "alpha.git");
    let work = tempfile::tempdir().unwrap();
    let work_path = work.path();
    common::git(
        work_path,
        &["clone", "--quiet", bare.to_str().unwrap(), "."],
    );
    std::fs::write(work_path.join("a.txt"), "one\n").unwrap();
    let start = commit_all(work_path, "feat: base");
    for i in 0..101 {
        common::git(
            work_path,
            &[
                "commit",
                "--quiet",
                "--allow-empty",
                "-m",
                &format!("chore: filler {i}"),
            ],
        );
    }
    common::git(work_path, &["push", "--quiet", "origin", "HEAD:main"]);

    let (status, json) = common::get_json(
        router_for(root.path()),
        &format!("/api/v1/repos/alpha/patch?from={start}&to=main"),
    )
    .await;
    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("invalid_param")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn patch_should_include_a_merge_against_its_first_parent() {
    let root = tempfile::tempdir().unwrap();
    let bare = common::create_bare_repo(root.path(), "alpha.git");
    let work = tempfile::tempdir().unwrap();
    let work_path = work.path();
    common::git(
        work_path,
        &["clone", "--quiet", bare.to_str().unwrap(), "."],
    );
    std::fs::write(work_path.join("base.txt"), "base\n").unwrap();
    let start = commit_all(work_path, "feat: base");
    common::git(work_path, &["checkout", "--quiet", "-b", "feature"]);
    std::fs::write(work_path.join("feature.txt"), "feature\n").unwrap();
    commit_all(work_path, "feat: feature file");
    common::git(work_path, &["checkout", "--quiet", "main"]);
    std::fs::write(work_path.join("main.txt"), "main\n").unwrap();
    commit_all(work_path, "feat: main file");
    common::git(
        work_path,
        &[
            "merge",
            "--quiet",
            "--no-ff",
            "-m",
            "merge feature",
            "feature",
        ],
    );
    let merge_sha = common::git_output(work_path, &["rev-parse", "HEAD"], &[]);
    common::git(work_path, &["push", "--quiet", "origin", "HEAD:main"]);

    let (status, _headers, body) = get_bytes(
        root.path(),
        &format!("/api/v1/repos/alpha/patch?from={start}&to={merge_sha}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let text = String::from_utf8(body).unwrap();
    // Three commits in (start, merge_sha]: the feature-file commit, the
    // main-file commit, and the merge itself — plain `git format-patch`
    // would skip the merge entirely; axgit includes it as its own block,
    // diffed against its first parent, consistent with every other diff in
    // this API being first-parent-only.
    let block_count = text.matches("Mon Sep 17 00:00:00 2001").count();
    assert_eq!(block_count, 3, "{text}");
    assert!(
        text.contains("Subject: [PATCH 3/3] merge feature"),
        "{text}"
    );
    assert!(text.contains("main.txt"), "{text}");
    // The merge's own first-parent diff shows feature.txt as newly added
    // relative to main's tip (it was merged in from the second parent).
    assert!(text.contains("feature.txt"), "{text}");
}

#[tokio::test]
async fn patch_and_rawdiff_should_404_on_unknown_refs() {
    let (root, _shas) = setup();
    for endpoint in ["patch", "rawdiff"] {
        let (status, json) = common::get_json(
            router_for(root.path()),
            &format!("/api/v1/repos/alpha/{endpoint}?to=no-such-ref"),
        )
        .await;
        assert_eq!(
            (status, json["error"]["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("ref_not_found")),
            "unexpected response for {endpoint}: {json}"
        );
    }
}

#[tokio::test]
async fn patch_and_rawdiff_should_be_immutable_for_full_shas() {
    let (root, shas) = setup();
    for endpoint in ["patch", "rawdiff"] {
        let (status, headers, _body) = get_bytes(
            root.path(),
            &format!(
                "/api/v1/repos/alpha/{endpoint}?from={}&to={}",
                shas[0], shas[1]
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "endpoint {endpoint}");
        assert_eq!(
            headers
                .get("cache-control")
                .map(|value| value.to_str().unwrap()),
            Some("public, max-age=31536000, immutable"),
            "endpoint {endpoint}"
        );
        assert!(!headers.contains_key("etag"), "endpoint {endpoint}");

        let (status, headers, _body) = get_bytes(
            root.path(),
            &format!("/api/v1/repos/alpha/{endpoint}?to=main"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "endpoint {endpoint}");
        assert_eq!(
            headers
                .get("cache-control")
                .map(|value| value.to_str().unwrap()),
            Some("no-cache"),
            "endpoint {endpoint}"
        );
        assert!(headers.contains_key("etag"), "endpoint {endpoint}");
    }
}
