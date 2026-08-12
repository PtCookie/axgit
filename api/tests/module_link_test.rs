//! Integration tests for submodule (gitlink) `module_link` resolution —
//! cgit's `module-link` / `repo.module-link.<path>` parity
//! (docs/DECISIONS.md #72). Unit-level template/parser edge cases live in
//! `api/src/repo/submodule.rs`'s own `mod tests`; this file exercises the
//! config → `.gitmodules` precedence and the two config key levels end to
//! end, through the real router.

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

/// `repo.git`: three gitlinks and a `.gitmodules`.
///
/// - `dep` (root) — mapped in `.gitmodules` under a *different* stanza name
///   (`dep-name`), to prove the lookup keys on `path`, not the stanza name.
///   `.gitmodules` gives it an http(s) `url`.
/// - `ssh-dep` (root) — mapped in `.gitmodules` to a `git@…` URL, which
///   `is_http_url` rejects.
/// - `third_party/nested/lib` — **not** in `.gitmodules` at all, so it's a
///   clean target for the repo-wide config-template substitution test (the
///   substituted path must be the full `third_party/nested/lib`, not `lib`).
struct Fixture {
    root: TempDir,
    bare: std::path::PathBuf,
    dep_sha: String,
    ssh_dep_sha: String,
    nested_sha: String,
}

fn setup() -> Fixture {
    let root = tempfile::tempdir().expect("failed to create fixture root");
    let bare = common::create_bare_repo(root.path(), "repo.git");

    let work = tempfile::tempdir().expect("failed to create work dir");
    let w = work.path();
    common::git(w, &["clone", "--quiet", bare.to_str().unwrap(), "."]);

    let dep_sha = "a".repeat(40);
    let ssh_dep_sha = "b".repeat(40);
    let nested_sha = "c".repeat(40);

    std::fs::write(
        w.join(".gitmodules"),
        r#"[submodule "dep-name"]
	path = dep
	url = https://example.com/dep.git
[submodule "ssh-name"]
	path = ssh-dep
	url = git@host:a/b.git
"#,
    )
    .unwrap();
    // Empty directories at each gitlink path keep `git add -A` from staging
    // (and then immediately conflicting with) the gitlink's own removal —
    // same trick `files_test.rs` uses.
    std::fs::create_dir_all(w.join("dep")).unwrap();
    std::fs::create_dir_all(w.join("ssh-dep")).unwrap();
    std::fs::create_dir_all(w.join("third_party/nested/lib")).unwrap();
    common::git(w, &["add", "-A"]);
    for (sha, path) in [
        (&dep_sha, "dep"),
        (&ssh_dep_sha, "ssh-dep"),
        (&nested_sha, "third_party/nested/lib"),
    ] {
        common::git(
            w,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{sha},{path}"),
            ],
        );
    }
    common::git(
        w,
        &["commit", "--quiet", "-m", "feat: gitlinks + .gitmodules"],
    );
    common::git(w, &["push", "--quiet", "origin", "HEAD:main"]);

    Fixture {
        root,
        bare,
        dep_sha,
        ssh_dep_sha,
        nested_sha,
    }
}

fn entry<'a>(json: &'a Value, name: &str) -> &'a Value {
    json["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == name)
        .unwrap_or_else(|| panic!("missing entry {name} in {json}"))
}

#[tokio::test]
async fn gitmodules_url_is_used_when_no_config_is_set() {
    let fixture = setup();
    let json = get_ok(fixture.root.path(), "/api/v1/repos/repo/tree/main").await;
    assert_eq!(
        entry(&json, "dep")["module_link"],
        "https://example.com/dep.git",
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn gitmodules_non_http_url_yields_no_link() {
    let fixture = setup();
    let json = get_ok(fixture.root.path(), "/api/v1/repos/repo/tree/main").await;
    let ssh_dep = entry(&json, "ssh-dep");
    assert_eq!(
        ssh_dep["sha"], fixture.ssh_dep_sha,
        "unexpected response: {json}"
    );
    assert_eq!(
        ssh_dep["module_link"],
        Value::Null,
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn a_gitlink_absent_from_gitmodules_and_config_has_no_link() {
    let fixture = setup();
    let json = get_ok(
        fixture.root.path(),
        "/api/v1/repos/repo/tree/main/third_party/nested",
    )
    .await;
    assert_eq!(
        entry(&json, "lib")["module_link"],
        Value::Null,
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn repo_wide_template_substitutes_the_full_path_for_a_nested_gitlink() {
    let fixture = setup();
    common::set_meta(
        &fixture.bare,
        "cgit",
        "module-link",
        "https://example.com/%s/commit/%s",
    );

    let json = get_ok(
        fixture.root.path(),
        "/api/v1/repos/repo/tree/main/third_party/nested",
    )
    .await;
    assert_eq!(
        entry(&json, "lib")["module_link"],
        format!(
            "https://example.com/third_party/nested/lib/commit/{}",
            fixture.nested_sha
        ),
        "the substituted path must be the full repo-relative path, not just the entry name: {json}"
    );
}

#[tokio::test]
async fn per_path_override_wins_for_one_gitlink_while_repo_wide_applies_to_another() {
    let fixture = setup();
    common::set_meta(
        &fixture.bare,
        "cgit",
        "module-link",
        "https://example.com/%s/commit/%s",
    );
    common::set_meta(
        &fixture.bare,
        "axgit",
        "dep.module-link",
        "https://override.example/only-dep",
    );

    let root = get_ok(fixture.root.path(), "/api/v1/repos/repo/tree/main").await;
    assert_eq!(
        entry(&root, "dep")["module_link"],
        "https://override.example/only-dep"
    );

    let nested = get_ok(
        fixture.root.path(),
        "/api/v1/repos/repo/tree/main/third_party/nested",
    )
    .await;
    assert_eq!(
        entry(&nested, "lib")["module_link"],
        format!(
            "https://example.com/third_party/nested/lib/commit/{}",
            fixture.nested_sha
        )
    );
}

#[tokio::test]
async fn config_template_wins_over_gitmodules_when_both_apply() {
    let fixture = setup();
    // `dep` already has a `.gitmodules` mapping; a config template for the
    // same path must take priority, not fall through to it.
    common::set_meta(
        &fixture.bare,
        "axgit",
        "dep.module-link",
        "https://config-wins.example/dep",
    );

    let json = get_ok(fixture.root.path(), "/api/v1/repos/repo/tree/main").await;
    assert_eq!(
        entry(&json, "dep")["module_link"],
        "https://config-wins.example/dep"
    );
}

#[tokio::test]
async fn an_empty_per_path_value_suppresses_the_repo_wide_template() {
    let fixture = setup();
    common::set_meta(
        &fixture.bare,
        "cgit",
        "module-link",
        "https://example.com/%s/commit/%s",
    );
    common::set_meta(&fixture.bare, "axgit", "dep.module-link", "");

    let json = get_ok(fixture.root.path(), "/api/v1/repos/repo/tree/main").await;
    assert_eq!(
        entry(&json, "dep")["module_link"],
        Value::Null,
        "an empty per-path template suppresses the link entirely, it does not fall back: {json}"
    );

    // The other gitlink is unaffected — the repo-wide template still applies.
    let nested = get_ok(
        fixture.root.path(),
        "/api/v1/repos/repo/tree/main/third_party/nested",
    )
    .await;
    assert_eq!(
        entry(&nested, "lib")["module_link"],
        format!(
            "https://example.com/third_party/nested/lib/commit/{}",
            fixture.nested_sha
        )
    );
}

#[tokio::test]
async fn a_javascript_scheme_template_yields_no_link() {
    let fixture = setup();
    common::set_meta(
        &fixture.bare,
        "axgit",
        "dep.module-link",
        "javascript:alert(1)",
    );

    let json = get_ok(fixture.root.path(), "/api/v1/repos/repo/tree/main").await;
    assert_eq!(
        entry(&json, "dep")["module_link"],
        Value::Null,
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn a_bare_path_only_template_yields_no_link() {
    let fixture = setup();
    // `%s` alone expands to the entry's own path (e.g. "dep") — no scheme,
    // no leading slash, so `is_link_href` rejects it. The guard runs on the
    // *expanded* string, not the template.
    common::set_meta(&fixture.bare, "axgit", "dep.module-link", "%s");

    let json = get_ok(fixture.root.path(), "/api/v1/repos/repo/tree/main").await;
    assert_eq!(
        entry(&json, "dep")["module_link"],
        Value::Null,
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn a_root_relative_template_survives_intact() {
    let fixture = setup();
    common::set_meta(
        &fixture.bare,
        "cgit",
        "module-link",
        "/git/%s/commit/?id=%s",
    );

    let json = get_ok(fixture.root.path(), "/api/v1/repos/repo/tree/main").await;
    assert_eq!(
        entry(&json, "dep")["module_link"],
        format!("/git/dep/commit/?id={}", fixture.dep_sha),
        "unexpected response: {json}"
    );
}

#[tokio::test]
async fn the_by_oid_tree_always_reports_null_even_with_config_set() {
    let fixture = setup();
    common::set_meta(
        &fixture.bare,
        "cgit",
        "module-link",
        "https://example.com/%s/commit/%s",
    );

    let root_tree_sha = common::git_output(&fixture.bare, &["rev-parse", "main^{tree}"], &[]);
    let json = get_ok(
        fixture.root.path(),
        &format!("/api/v1/repos/repo/objects/{root_tree_sha}"),
    )
    .await;

    let dep = entry(&json["tree"], "dep");
    assert_eq!(dep["sha"], fixture.dep_sha);
    assert_eq!(
        dep["module_link"],
        Value::Null,
        "the by-oid tree has no path context to resolve a template against: {json}"
    );
}

#[tokio::test]
async fn a_non_gitlink_entry_reports_no_module_link() {
    let fixture = setup();
    common::set_meta(
        &fixture.bare,
        "cgit",
        "module-link",
        "https://example.com/%s/commit/%s",
    );

    // `third_party` is a directory, not a gitlink, sitting in the same
    // listing as a real gitlink (`dep`) — its `module_link` must stay `null`
    // even though a repo-wide template is configured.
    let json = get_ok(fixture.root.path(), "/api/v1/repos/repo/tree/main").await;
    assert_eq!(
        entry(&json, "third_party")["module_link"],
        Value::Null,
        "unexpected response: {json}"
    );
    assert!(
        entry(&json, "dep")["module_link"].is_string(),
        "sanity check: the gitlink in the same listing does get a link: {json}"
    );
}
