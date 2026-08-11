//! The generated OpenAPI document is committed to `docs/openapi.json` so that
//! spec changes show up in review diffs and the frontend can generate its
//! types from a checked-in file. These tests guard that snapshot and the
//! routes that serve it.

mod common;

use std::collections::BTreeSet;
use std::path::PathBuf;

use axum::http::StatusCode;
use serde_json::Value;
use utoipa::OpenApi;

use axgit::openapi::ApiDoc;
use axgit::routes::{OPENAPI_JSON_PATH, SWAGGER_UI_PATH};

use common::{get_bytes_with_headers, router_for};

/// Set to rewrite `docs/openapi.json` instead of failing on a mismatch.
const UPDATE_ENV: &str = "AXGIT_UPDATE_OPENAPI";

/// Every path+method the spec is expected to describe. Adding an endpoint
/// without a `#[utoipa::path]` annotation (or vice versa) fails here.
const EXPECTED_OPERATIONS: &[(&str, &str)] = &[
    ("/api/v1/repos", "get"),
    ("/api/v1/repos/{repo}", "get"),
    ("/api/v1/repos/{repo}/refs", "get"),
    ("/api/v1/repos/{repo}/tags/{name}", "get"),
    ("/api/v1/repos/{repo}/objects/{oid}", "get"),
    ("/api/v1/repos/{repo}/objects/{oid}/raw", "get"),
    ("/api/v1/repos/{repo}/commits", "get"),
    ("/api/v1/repos/{repo}/commits/{sha}", "get"),
    ("/api/v1/repos/{repo}/commits/{sha}/diff", "get"),
    ("/api/v1/repos/{repo}/diff", "get"),
    ("/api/v1/repos/{repo}/rawdiff", "get"),
    ("/api/v1/repos/{repo}/patch", "get"),
    ("/api/v1/repos/{repo}/tree/{ref}/{path}", "get"),
    ("/api/v1/repos/{repo}/blob/{ref}/{path}", "get"),
    ("/api/v1/repos/{repo}/raw/{ref}/{path}", "get"),
    ("/api/v1/repos/{repo}/readme", "get"),
    ("/api/v1/repos/{repo}/blame/{ref}/{path}", "get"),
    ("/api/v1/repos/{repo}/archive/{ref}.{format}", "get"),
    ("/api/v1/repos/{repo}/feed.atom", "get"),
    ("/api/v1/repos/{repo}/search", "get"),
    ("/api/v1/repos/{repo}/stats", "get"),
    ("/{repo_git}/info/refs", "get"),
    ("/{repo_git}/git-upload-pack", "post"),
    ("/{repo_git}/git-receive-pack", "post"),
];

fn snapshot_path() -> PathBuf {
    // CARGO_MANIFEST_DIR is `api/`; the spec lives next to the prose contract.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../docs/openapi.json")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/openapi.json"))
}

/// Trailing newline included so the file is POSIX-clean and diffs stay small.
fn generated() -> String {
    format!("{}\n", ApiDoc::openapi().to_pretty_json().unwrap())
}

#[test]
fn openapi_snapshot_should_match_the_committed_document() {
    let path = snapshot_path();
    let generated = generated();

    if std::env::var_os(UPDATE_ENV).is_some() {
        std::fs::write(&path, &generated).expect("failed to write docs/openapi.json");
        return;
    }

    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        committed, generated,
        "docs/openapi.json is out of date; regenerate with\n    \
         {UPDATE_ENV}=1 cargo test --manifest-path api/Cargo.toml --test openapi_test",
    );
}

#[test]
fn openapi_should_describe_exactly_the_routed_operations() {
    let spec: Value = serde_json::from_str(&generated()).unwrap();
    let paths = spec["paths"].as_object().expect("spec has paths");

    let described: BTreeSet<(String, String)> = paths
        .iter()
        .flat_map(|(path, item)| {
            item.as_object()
                .expect("path item is an object")
                .keys()
                .map(move |method| (path.clone(), method.clone()))
        })
        .collect();
    let expected: BTreeSet<(String, String)> = EXPECTED_OPERATIONS
        .iter()
        .map(|(path, method)| ((*path).to_owned(), (*method).to_owned()))
        .collect();

    assert_eq!(described, expected);
}

#[tokio::test]
async fn openapi_json_route_should_serve_the_document() {
    let dir = tempfile::tempdir().unwrap();
    let (status, headers, body) =
        get_bytes_with_headers(router_for(dir.path()), OPENAPI_JSON_PATH).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "application/json");
    let served: Value = serde_json::from_slice(&body).expect("response body is not JSON");
    assert!(served["openapi"].as_str().unwrap().starts_with("3.1"));
    assert_eq!(served["info"]["title"], "Axgit API");
}

#[tokio::test]
async fn swagger_ui_should_be_served_from_the_vendored_assets() {
    let dir = tempfile::tempdir().unwrap();
    // The bare path redirects to the trailing-slash form, which serves the page.
    let (status, headers, _) =
        get_bytes_with_headers(router_for(dir.path()), SWAGGER_UI_PATH).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers["location"], "/swagger-ui/");

    let uri = format!("{SWAGGER_UI_PATH}/");
    let (status, _, body) = get_bytes_with_headers(router_for(dir.path()), &uri).await;
    assert_eq!(status, StatusCode::OK);
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("swagger-ui"), "unexpected Swagger UI page");
}
