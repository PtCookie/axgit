mod common;

use common::router_for;

use std::path::Path;

use axgit::repo::sort::RepoOrder;
use axgit::routes::build_router;
use axgit::state::AppState;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tempfile::TempDir;
use tower::ServiceExt;

/// Fixture set: `alpha.git` ([cgit] meta + agefile + commit),
/// `bravo.git` ([axgit] overriding [cgit], no agefile, commit),
/// `empty.git` (no commits, no meta).
fn setup_fixtures() -> TempDir {
    let root = tempfile::tempdir().expect("failed to create fixture root");

    let alpha = common::create_bare_repo(root.path(), "alpha.git");
    common::set_meta(&alpha, "cgit", "section", "infra");
    common::set_meta(&alpha, "cgit", "owner", "PtCookie");
    common::set_meta(&alpha, "cgit", "desc", "Alpha repository");
    common::add_commit(&alpha, "README.md", "# alpha\n", "feat: initial commit");
    common::write_agefile(&alpha, "2026-07-24 13:06:00 +0900\n");

    let bravo = common::create_bare_repo(root.path(), "bravo.git");
    common::set_meta(&bravo, "cgit", "desc", "cgit description");
    common::set_meta(&bravo, "axgit", "desc", "axgit description");
    common::add_commit(&bravo, "README.md", "# bravo\n", "feat: initial commit");

    common::create_bare_repo(root.path(), "empty.git");

    root
}

async fn list_repos(root: &Path) -> Value {
    let (status, json) = common::get_json(router_for(root), "/api/v1/repos").await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");
    json
}

#[tokio::test]
async fn list_repos_returns_repos_sorted_by_name() {
    let root = setup_fixtures();

    let json = list_repos(root.path()).await;

    let names: Vec<&str> = json["repos"]
        .as_array()
        .expect("repos is not an array")
        .iter()
        .map(|repo| repo["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["alpha", "bravo", "empty"]);
}

#[tokio::test]
async fn list_repos_reads_cgit_metadata() {
    let root = setup_fixtures();

    let json = list_repos(root.path()).await;

    let alpha = &json["repos"][0];
    assert_eq!(
        (
            alpha["section"].as_str(),
            alpha["owner"].as_str(),
            alpha["description"].as_str(),
            alpha["default_branch"].as_str(),
        ),
        (
            Some("infra"),
            Some("PtCookie"),
            Some("Alpha repository"),
            Some("main")
        ),
    );
}

#[tokio::test]
async fn list_repos_prefers_axgit_section_over_cgit() {
    let root = setup_fixtures();

    let json = list_repos(root.path()).await;

    assert_eq!(json["repos"][1]["description"], "axgit description");
}

#[tokio::test]
async fn list_repos_reads_last_modified_from_agefile() {
    let root = setup_fixtures();

    let json = list_repos(root.path()).await;

    assert_eq!(
        json["repos"][0]["last_modified"],
        "2026-07-24T13:06:00+09:00"
    );
}

#[tokio::test]
async fn list_repos_falls_back_to_head_authordate_without_agefile() {
    let root = setup_fixtures();

    let json = list_repos(root.path()).await;

    assert_eq!(json["repos"][1]["last_modified"], common::FIXED_DATE);
}

#[tokio::test]
async fn list_repos_returns_null_fields_for_empty_repo() {
    let root = setup_fixtures();

    let json = list_repos(root.path()).await;

    let empty = &json["repos"][2];
    assert_eq!(
        (
            empty["default_branch"].clone(),
            empty["last_modified"].clone()
        ),
        (Value::Null, Value::Null),
    );
}

#[tokio::test]
async fn list_repos_defaults_to_name_order_and_echoes_the_effective_sort() {
    let root = setup_fixtures();

    let json = list_repos(root.path()).await;

    assert_eq!(json["sort"], "name");
}

#[tokio::test]
async fn list_repos_sort_owner_places_a_missing_owner_last() {
    let root = setup_fixtures();

    let (status, json) =
        common::get_json(router_for(root.path()), "/api/v1/repos?sort=owner").await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");

    let names: Vec<&str> = json["repos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|repo| repo["name"].as_str().unwrap())
        .collect();
    // alpha has owner "PtCookie"; bravo/empty have none and fall back to the
    // name-ascending tiebreak, last regardless of direction.
    assert_eq!(names, ["alpha", "bravo", "empty"]);
    assert_eq!(json["sort"], "owner");
}

#[tokio::test]
async fn list_repos_sort_reversed_owner_still_places_a_missing_owner_last() {
    let root = setup_fixtures();

    let (status, json) =
        common::get_json(router_for(root.path()), "/api/v1/repos?sort=-owner").await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");

    let names: Vec<&str> = json["repos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|repo| repo["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["alpha", "bravo", "empty"]);
    assert_eq!(json["sort"], "-owner");
}

#[tokio::test]
async fn list_repos_sort_idle_defaults_to_descending_newest_first() {
    let root = setup_fixtures();

    let (status, json) = common::get_json(router_for(root.path()), "/api/v1/repos?sort=idle").await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");

    let names: Vec<&str> = json["repos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|repo| repo["name"].as_str().unwrap())
        .collect();
    // alpha: 2026-07-24 (agefile); bravo: FIXED_DATE = 2026-07-01 (authordate
    // fallback); empty: null, always last.
    assert_eq!(names, ["alpha", "bravo", "empty"]);
    assert_eq!(json["sort"], "idle");
}

#[tokio::test]
async fn list_repos_honours_the_configured_server_default_sort() {
    let root = setup_fixtures();

    let mut config = common::test_config(root.path());
    config.repository_sort = RepoOrder::parse("owner").unwrap();
    let router = build_router(AppState::new(config));

    let (status, json) = common::get_json(router, "/api/v1/repos").await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");
    assert_eq!(json["sort"], "owner");
}

#[tokio::test]
async fn list_repos_reads_a_valid_homepage_url() {
    let root = setup_fixtures();
    common::set_meta(
        &root.path().join("alpha.git"),
        "cgit",
        "homepage",
        "https://example.com/alpha",
    );

    let json = list_repos(root.path()).await;

    assert_eq!(json["repos"][0]["homepage"], "https://example.com/alpha");
}

#[tokio::test]
async fn list_repos_drops_a_non_http_homepage_scheme() {
    let root = setup_fixtures();
    common::set_meta(
        &root.path().join("alpha.git"),
        "cgit",
        "homepage",
        "javascript:alert(1)",
    );

    let json = list_repos(root.path()).await;

    assert_eq!(
        json["repos"][0]["homepage"],
        Value::Null,
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn list_repos_treats_blank_metadata_as_unset() {
    let root = setup_fixtures();
    // alpha.git already has non-blank section/owner/desc from setup_fixtures — overwrite them
    // with blank (empty and whitespace-only) values, distinct from the key being absent
    // entirely (which `empty.git` already covers).
    common::set_meta(&root.path().join("alpha.git"), "cgit", "section", "");
    common::set_meta(&root.path().join("alpha.git"), "cgit", "owner", "   ");
    common::set_meta(&root.path().join("alpha.git"), "cgit", "desc", "");

    let json = list_repos(root.path()).await;

    let alpha = &json["repos"][0];
    assert_eq!(
        (
            alpha["section"].as_str(),
            alpha["owner"].as_str(),
            alpha["description"].as_str(),
        ),
        (None, None, None),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn list_repos_rejects_an_unknown_sort() {
    let root = setup_fixtures();

    let (status, json) =
        common::get_json(router_for(root.path()), "/api/v1/repos?sort=bogus").await;

    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("invalid_param")),
        "unexpected response: {json}"
    );
}

/// Fixture set for the `hide`/`ignore` flags (docs/DECISIONS.md #66):
/// `visible.git` (no flags), `hidden.git` (`cgit.hide`), `ignored.git`
/// (`cgit.ignore`). Kept separate from `setup_fixtures` so the positional
/// name-order assertions above don't have to account for two more repos.
fn setup_hide_ignore_fixtures() -> TempDir {
    let root = tempfile::tempdir().expect("failed to create fixture root");

    let visible = common::create_bare_repo(root.path(), "visible.git");
    common::add_commit(&visible, "README.md", "# visible\n", "feat: initial commit");

    let hidden = common::create_bare_repo(root.path(), "hidden.git");
    common::set_meta(&hidden, "cgit", "hide", "true");
    common::add_commit(&hidden, "README.md", "# hidden\n", "feat: initial commit");

    let ignored = common::create_bare_repo(root.path(), "ignored.git");
    common::set_meta(&ignored, "cgit", "ignore", "true");
    common::add_commit(&ignored, "README.md", "# ignored\n", "feat: initial commit");

    root
}

#[tokio::test]
async fn list_repos_omits_hidden_and_ignored_repositories() {
    let root = setup_hide_ignore_fixtures();

    let json = list_repos(root.path()).await;

    let names: Vec<&str> = json["repos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|repo| repo["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["visible"]);
}

#[tokio::test]
async fn hidden_repository_is_still_reachable_by_direct_path() {
    let root = setup_hide_ignore_fixtures();

    let (status, json) = common::get_json(router_for(root.path()), "/api/v1/repos/hidden").await;

    assert_eq!(status, StatusCode::OK, "unexpected response: {json}");
    assert_eq!(json["name"], "hidden");
}

#[tokio::test]
async fn ignored_repository_is_not_reachable_by_direct_path() {
    let root = setup_hide_ignore_fixtures();

    let (status, json) = common::get_json(router_for(root.path()), "/api/v1/repos/ignored").await;

    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("repo_not_found")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn ignored_repository_refs_are_also_not_reachable() {
    let root = setup_hide_ignore_fixtures();

    let (status, json) =
        common::get_json(router_for(root.path()), "/api/v1/repos/ignored/refs").await;

    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("repo_not_found")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn ignored_repository_clone_is_also_not_reachable() {
    let root = setup_hide_ignore_fixtures();

    let (status, json) = common::get_json(
        router_for(root.path()),
        "/ignored.git/info/refs?service=git-upload-pack",
    )
    .await;

    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("repo_not_found")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn receive_pack_returns_403_read_only() {
    let root = setup_fixtures();

    let response = router_for(root.path())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/alpha.git/git-receive-pack")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("read_only")),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn info_refs_with_receive_pack_service_returns_403() {
    let root = setup_fixtures();

    let (status, json) = common::get_json(
        router_for(root.path()),
        "/alpha.git/info/refs?service=git-receive-pack",
    )
    .await;

    assert_eq!(
        (status, json["error"]["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("read_only")),
        "unexpected response: {json}"
    );
}
