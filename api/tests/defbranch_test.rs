//! Integration tests for the configured `defbranch` (docs/DECISIONS.md #68)
//! — every "no ref given" site resolves the literal `HEAD` through it, via
//! `repo::resolve::resolve_commit`'s substitution.

mod common;

use std::path::Path;

use axum::http::StatusCode;
use serde_json::Value;
use tempfile::TempDir;

use common::router_for;

async fn get_ok(root: &Path, uri: &str) -> Value {
    let (status, json) = common::get_json(router_for(root), uri).await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");
    json
}

/// `repo.git`: two commits on `main` (`c1`, `c2`), and a `release` branch
/// created right after `c1` — before `c2` existed — so `main` and `release`
/// diverge. `main` (HEAD) has `a.txt` + `b.txt`; `release` has only `a.txt`.
/// Returns `(root, c1, c2)`.
fn setup_diverging_fixture() -> (TempDir, String, String) {
    let root = tempfile::tempdir().expect("failed to create fixture root");
    let bare = common::create_bare_repo(root.path(), "repo.git");

    let shas = common::commit_history(
        &bare,
        &[common::CommitSpec {
            file: "a.txt",
            content: "a\n",
            message: "feat: add a",
            date: common::FIXED_DATE,
        }],
    );
    let c1 = shas[0].clone();
    common::add_branch(&bare, "release");

    let shas = common::commit_history(
        &bare,
        &[common::CommitSpec {
            file: "b.txt",
            content: "b\n",
            message: "feat: add b",
            date: common::FIXED_DATE,
        }],
    );
    let c2 = shas[0].clone();

    (root, c1, c2)
}

#[tokio::test]
async fn defbranch_changes_the_summarys_default_branch_and_head() {
    let (root, c1, _c2) = setup_diverging_fixture();
    common::set_meta(
        &root.path().join("repo.git"),
        "cgit",
        "defbranch",
        "release",
    );

    let json = get_ok(root.path(), "/api/v1/repos/repo").await;

    assert_eq!(
        (json["default_branch"].as_str(), json["head"].as_str()),
        (Some("release"), Some(c1.as_str())),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn defbranch_changes_the_ref_less_commit_log() {
    let (root, c1, _c2) = setup_diverging_fixture();
    common::set_meta(
        &root.path().join("repo.git"),
        "cgit",
        "defbranch",
        "release",
    );

    let json = get_ok(root.path(), "/api/v1/repos/repo/commits").await;

    let shas: Vec<&str> = json["commits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|commit| commit["sha"].as_str().unwrap())
        .collect();
    assert_eq!(shas, [c1.as_str()], "unexpected response: {json}");
}

#[tokio::test]
async fn defbranch_changes_the_ref_less_tree() {
    let (root, _c1, _c2) = setup_diverging_fixture();
    common::set_meta(
        &root.path().join("repo.git"),
        "cgit",
        "defbranch",
        "release",
    );

    let json = get_ok(root.path(), "/api/v1/repos/repo/tree/HEAD").await;

    let names: Vec<&str> = json["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["a.txt"], "unexpected response: {json}");
}

#[tokio::test]
async fn an_unset_defbranch_leaves_head_unchanged() {
    let (root, _c1, c2) = setup_diverging_fixture();

    let json = get_ok(root.path(), "/api/v1/repos/repo").await;

    assert_eq!(
        (json["default_branch"].as_str(), json["head"].as_str()),
        (Some("main"), Some(c2.as_str())),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn a_defbranch_naming_a_nonexistent_branch_falls_back_to_head() {
    let (root, _c1, c2) = setup_diverging_fixture();
    common::set_meta(
        &root.path().join("repo.git"),
        "cgit",
        "defbranch",
        "no-such-branch",
    );

    let json = get_ok(root.path(), "/api/v1/repos/repo").await;

    assert_eq!(
        (json["default_branch"].as_str(), json["head"].as_str()),
        (Some("main"), Some(c2.as_str())),
        "unexpected response: {json}"
    );
}
