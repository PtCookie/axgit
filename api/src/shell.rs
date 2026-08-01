//! Maps request paths onto the prerendered Astro page shells.
//!
//! The static build has one HTML file per *route shape*, not per repository —
//! the repository list is per-deployment and unknown at build time — so
//! `web/src/pages/[repo]/` is built once under a reserved placeholder param
//! and this module rewrites request paths onto those files
//! (docs/DECISIONS.md #17).
//!
//! Mirrors `web/src/lib/shell.ts::shellFor`, which the Astro dev server uses
//! for the same purpose. The two must change together.

use std::path::{Path, PathBuf};

use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};

/// The reserved `getStaticPaths` param the `/{repo}` shells are built under.
/// Keep in sync with `web/src/lib/shell.ts::REPO_SHELL_PARAM`.
pub const REPO_SHELL_PARAM: &str = "__repo__";

/// Resolves a request path to its shell file (relative to the static build
/// root) and the status to answer with.
///
/// Only the *shape* of the path is inspected: the `{repo}` segment is
/// matched but never used to build a filesystem path, so percent-encoding or
/// `..` inside it are structurally harmless here.
fn shell_for(path: &str) -> (PathBuf, StatusCode) {
    let segments: Vec<&str> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();

    match segments.as_slice() {
        [] => (PathBuf::from("index.html"), StatusCode::OK),
        [_repo] => (
            Path::new(REPO_SHELL_PARAM).join("index.html"),
            StatusCode::OK,
        ),
        [_repo, "refs"] => (
            Path::new(REPO_SHELL_PARAM).join("refs").join("index.html"),
            StatusCode::OK,
        ),
        [_repo, "log"] => (
            Path::new(REPO_SHELL_PARAM).join("log").join("index.html"),
            StatusCode::OK,
        ),
        [_repo, "commit", _sha] => (
            Path::new(REPO_SHELL_PARAM)
                .join("commit")
                .join("index.html"),
            StatusCode::OK,
        ),
        // The path after `/tree/` is optional (empty means the root tree).
        [_repo, "tree", ..] => (
            Path::new(REPO_SHELL_PARAM).join("tree").join("index.html"),
            StatusCode::OK,
        ),
        // At least one path segment is required — there's nothing to show
        // for `/{repo}/blob` itself.
        [_repo, "blob", _first, ..] => (
            Path::new(REPO_SHELL_PARAM).join("blob").join("index.html"),
            StatusCode::OK,
        ),
        _ => (PathBuf::from("404.html"), StatusCode::NOT_FOUND),
    }
}

/// `ServeDir` fallback: a request with no matching file gets the page shell
/// for its route shape, or the 404 shell — with a real 404 — if there is
/// none.
pub async fn serve_shell(static_dir: PathBuf, uri: Uri) -> Response {
    let (relative, status) = shell_for(uri.path());
    let file = static_dir.join(relative);

    match tokio::fs::read(&file).await {
        Ok(body) => (
            status,
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                // The shell is rebuilt on every deploy and is tiny; never let
                // a browser hold a stale one against fresh `_astro/` hashes.
                (header::CACHE_CONTROL, "no-cache"),
            ],
            body,
        )
            .into_response(),
        Err(error) => {
            tracing::error!(
                file = %file.display(),
                %error,
                "page shell missing from the static build",
            );
            StatusCode::NOT_FOUND.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_maps_to_the_index_shell() {
        assert_eq!(
            shell_for("/"),
            (PathBuf::from("index.html"), StatusCode::OK)
        );
    }

    #[test]
    fn repo_paths_map_to_the_repo_shell() {
        for path in ["/git-compose", "/git-compose/", "/my%20repo"] {
            assert_eq!(
                shell_for(path),
                (
                    Path::new(REPO_SHELL_PARAM).join("index.html"),
                    StatusCode::OK
                ),
                "path {path}"
            );
        }
    }

    #[test]
    fn repo_refs_paths_map_to_the_refs_shell() {
        for path in ["/git-compose/refs", "/git-compose/refs/"] {
            assert_eq!(
                shell_for(path),
                (
                    Path::new(REPO_SHELL_PARAM).join("refs").join("index.html"),
                    StatusCode::OK
                ),
                "path {path}"
            );
        }
    }

    #[test]
    fn repo_log_paths_map_to_the_log_shell() {
        for path in ["/git-compose/log", "/git-compose/log/"] {
            assert_eq!(
                shell_for(path),
                (
                    Path::new(REPO_SHELL_PARAM).join("log").join("index.html"),
                    StatusCode::OK
                ),
                "path {path}"
            );
        }
    }

    #[test]
    fn repo_commit_paths_map_to_the_commit_shell() {
        for path in ["/git-compose/commit/abc123", "/git-compose/commit/abc123/"] {
            assert_eq!(
                shell_for(path),
                (
                    Path::new(REPO_SHELL_PARAM)
                        .join("commit")
                        .join("index.html"),
                    StatusCode::OK
                ),
                "path {path}"
            );
        }
    }

    #[test]
    fn repo_tree_paths_map_to_the_tree_shell() {
        for path in [
            "/git-compose/tree",
            "/git-compose/tree/",
            "/git-compose/tree/src",
            "/git-compose/tree/src/lib",
        ] {
            assert_eq!(
                shell_for(path),
                (
                    Path::new(REPO_SHELL_PARAM).join("tree").join("index.html"),
                    StatusCode::OK
                ),
                "path {path}"
            );
        }
    }

    #[test]
    fn repo_blob_paths_map_to_the_blob_shell() {
        for path in [
            "/git-compose/blob/src/main.rs",
            "/git-compose/blob/src/main.rs/",
            "/git-compose/blob/README.md",
        ] {
            assert_eq!(
                shell_for(path),
                (
                    Path::new(REPO_SHELL_PARAM).join("blob").join("index.html"),
                    StatusCode::OK
                ),
                "path {path}"
            );
        }
    }

    #[test]
    fn unmatched_shapes_map_to_the_404_shell() {
        for path in [
            "/git-compose/blob",
            "/git-compose/blob/",
            "/git-compose/commit",
            "/git-compose/commit/abc123/extra",
            "/git-compose/stats",
            "/a/b/c",
        ] {
            assert_eq!(
                shell_for(path),
                (PathBuf::from("404.html"), StatusCode::NOT_FOUND),
                "path {path}"
            );
        }
    }
}
