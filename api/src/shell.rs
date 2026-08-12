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
use axum::response::{IntoResponse, Redirect, Response};

use crate::cgit_compat;
use crate::escape::xml_escape;

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
        [_repo, "search"] => (
            Path::new(REPO_SHELL_PARAM)
                .join("search")
                .join("index.html"),
            StatusCode::OK,
        ),
        [_repo, "stats"] => (
            Path::new(REPO_SHELL_PARAM).join("stats").join("index.html"),
            StatusCode::OK,
        ),
        // The compare page never carries the revisions in the path (they're
        // `?from=`/`?to=` query params, since a ref may itself contain `/`)
        // — a bare 2-segment shape is the whole story, unlike commit's.
        [_repo, "diff"] => (
            Path::new(REPO_SHELL_PARAM).join("diff").join("index.html"),
            StatusCode::OK,
        ),
        [_repo, "commit", _sha] => (
            Path::new(REPO_SHELL_PARAM)
                .join("commit")
                .join("index.html"),
            StatusCode::OK,
        ),
        // An oid is a single fixed segment, never containing `/` — same
        // exactly-3-segment shape as commit.
        [_repo, "object", _oid] => (
            Path::new(REPO_SHELL_PARAM)
                .join("object")
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
        // Same "at least one path segment" rule as blob — there's nothing to
        // blame without a file.
        [_repo, "blame", _first, ..] => (
            Path::new(REPO_SHELL_PARAM).join("blame").join("index.html"),
            StatusCode::OK,
        ),
        // Same "at least one segment" rule as blob/blame — a tag name may
        // itself contain `/`, and there's nothing to show for `/{repo}/tag`
        // itself (the refs page already is the tag listing).
        [_repo, "tag", _first, ..] => (
            Path::new(REPO_SHELL_PARAM).join("tag").join("index.html"),
            StatusCode::OK,
        ),
        _ => (PathBuf::from("404.html"), StatusCode::NOT_FOUND),
    }
}

/// `ServeDir` fallback, cgit-compatibility redirects included: a `.git`-suffixed
/// or cgit-query-shaped path (`docs/DECISIONS.md #35`) gets a permanent
/// redirect to its axgit equivalent; everything else falls through to
/// [`serve_shell`] unchanged.
pub async fn serve_shell_or_redirect(
    static_dir: PathBuf,
    clone_url_base: Option<String>,
    uri: Uri,
) -> Response {
    match cgit_compat::redirect_for(&uri) {
        Some(location) => Redirect::permanent(&location).into_response(),
        None => serve_shell(static_dir, clone_url_base, uri).await,
    }
}

/// `ServeDir` fallback: a request with no matching file gets the page shell
/// for its route shape, or the 404 shell — with a real 404 — if there is
/// none. A `__repo__` shell additionally gets per-repository `<link>`s
/// injected into its `<head>` (docs/DECISIONS.md #63) — the shell itself is
/// prerendered once under the placeholder param and can't know the
/// repository name at build time.
pub async fn serve_shell(
    static_dir: PathBuf,
    clone_url_base: Option<String>,
    uri: Uri,
) -> Response {
    let (relative, status) = shell_for(uri.path());
    let file = static_dir.join(&relative);

    match tokio::fs::read(&file).await {
        Ok(body) => {
            let body = match repo_segment_for(uri.path(), &relative) {
                Some(segment) => inject_repo_head_links(body, segment, clone_url_base.as_deref()),
                None => body,
            };
            (
                status,
                [
                    (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                    // The shell is rebuilt on every deploy and is tiny; never
                    // let a browser hold a stale one against fresh `_astro/`
                    // hashes.
                    (header::CACHE_CONTROL, "no-cache"),
                ],
                body,
            )
                .into_response()
        }
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

/// The raw (still percent-encoded) first path segment, when `relative`
/// resolved to a shell under [`REPO_SHELL_PARAM`] — i.e. the request was for
/// some `/{repo}/...` shape, not `/`, `/404`, or another top-level route.
/// Not decoded: the segment is reused as-is to build both an API path and an
/// external URL, both of which want it percent-encoded exactly as the
/// browser sent it.
fn repo_segment_for<'u>(path: &'u str, relative: &Path) -> Option<&'u str> {
    if !relative.starts_with(REPO_SHELL_PARAM) {
        return None;
    }
    path.split('/').find(|segment| !segment.is_empty())
}

/// Inserts the Atom-discovery and `vcs-git` `<link>`s before the shell's
/// `</head>`, or returns `body` unchanged if it has none. Byte-oriented
/// (rather than parsing the HTML) — the shell is a small, controlled
/// document axgit itself produced, matching this file's and `feed.rs`'s
/// "hand-build small fixed documents" stance (docs/DECISIONS.md #12).
fn inject_repo_head_links(body: Vec<u8>, segment: &str, clone_url_base: Option<&str>) -> Vec<u8> {
    const HEAD_CLOSE: &[u8] = b"</head>";
    let Some(pos) = find_subslice(&body, HEAD_CLOSE) else {
        return body;
    };

    let links = repo_head_links(segment, clone_url_base);
    let mut out = Vec::with_capacity(body.len() + links.len());
    out.extend_from_slice(&body[..pos]);
    out.extend_from_slice(links.as_bytes());
    out.extend_from_slice(&body[pos..]);
    out
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Builds the `<link>` markup itself. Titles are fixed strings rather than
/// the repository name — `segment` is only available percent-encoded here,
/// and decoding it back to a display name would need machinery this
/// serve-path otherwise has no reason to carry. `rel="vcs-git"` is omitted
/// when no `clone_url_base` is configured, the same `null` rule
/// `handlers/repos.rs::get_repo`'s `clone_url` field already follows.
fn repo_head_links(segment: &str, clone_url_base: Option<&str>) -> String {
    let segment = xml_escape(segment);
    let mut links = format!(
        "<link rel=\"alternate\" type=\"application/atom+xml\" title=\"Recent commits\" href=\"/api/v1/repos/{segment}/feed.atom\">\
<link rel=\"alternate\" type=\"application/atom+xml\" title=\"Recent commits (all refs)\" href=\"/api/v1/repos/{segment}/feed.atom?all=1\">"
    );
    if let Some(base) = clone_url_base {
        let base = xml_escape(base.trim_end_matches('/'));
        links.push_str(&format!(
            "<link rel=\"vcs-git\" title=\"Git repository\" href=\"{base}/{segment}.git\">"
        ));
    }
    links
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
    fn repo_search_paths_map_to_the_search_shell() {
        for path in ["/git-compose/search", "/git-compose/search/"] {
            assert_eq!(
                shell_for(path),
                (
                    Path::new(REPO_SHELL_PARAM)
                        .join("search")
                        .join("index.html"),
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
    fn repo_blame_paths_map_to_the_blame_shell() {
        for path in [
            "/git-compose/blame/src/main.rs",
            "/git-compose/blame/src/main.rs/",
            "/git-compose/blame/README.md",
        ] {
            assert_eq!(
                shell_for(path),
                (
                    Path::new(REPO_SHELL_PARAM).join("blame").join("index.html"),
                    StatusCode::OK
                ),
                "path {path}"
            );
        }
    }

    #[test]
    fn repo_tag_paths_map_to_the_tag_shell() {
        for path in [
            "/git-compose/tag/v1.0.0",
            "/git-compose/tag/v1.0.0/",
            "/git-compose/tag/release/1.0",
        ] {
            assert_eq!(
                shell_for(path),
                (
                    Path::new(REPO_SHELL_PARAM).join("tag").join("index.html"),
                    StatusCode::OK
                ),
                "path {path}"
            );
        }
    }

    #[test]
    fn repo_object_paths_map_to_the_object_shell() {
        for path in ["/git-compose/object/abc123", "/git-compose/object/abc123/"] {
            assert_eq!(
                shell_for(path),
                (
                    Path::new(REPO_SHELL_PARAM)
                        .join("object")
                        .join("index.html"),
                    StatusCode::OK
                ),
                "path {path}"
            );
        }
    }

    #[test]
    fn repo_stats_paths_map_to_the_stats_shell() {
        for path in ["/git-compose/stats", "/git-compose/stats/"] {
            assert_eq!(
                shell_for(path),
                (
                    Path::new(REPO_SHELL_PARAM).join("stats").join("index.html"),
                    StatusCode::OK
                ),
                "path {path}"
            );
        }
    }

    #[test]
    fn repo_diff_paths_map_to_the_diff_shell() {
        for path in ["/git-compose/diff", "/git-compose/diff/"] {
            assert_eq!(
                shell_for(path),
                (
                    Path::new(REPO_SHELL_PARAM).join("diff").join("index.html"),
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
            "/git-compose/blame",
            "/git-compose/blame/",
            "/git-compose/commit",
            "/git-compose/commit/abc123/extra",
            "/git-compose/object",
            "/git-compose/object/",
            "/git-compose/object/abc123/extra",
            "/git-compose/stats/extra",
            "/git-compose/diff/extra",
            "/git-compose/tag",
            "/git-compose/tag/",
            "/a/b/c",
        ] {
            assert_eq!(
                shell_for(path),
                (PathBuf::from("404.html"), StatusCode::NOT_FOUND),
                "path {path}"
            );
        }
    }

    #[test]
    fn repo_segment_for_should_extract_the_raw_first_segment_of_a_repo_shell() {
        assert_eq!(
            repo_segment_for(
                "/git-compose/blob/src/main.rs",
                &Path::new(REPO_SHELL_PARAM).join("blob").join("index.html"),
            ),
            Some("git-compose")
        );
        // Still percent-encoded — never decoded here.
        assert_eq!(
            repo_segment_for(
                "/my%20repo",
                &Path::new(REPO_SHELL_PARAM).join("index.html"),
            ),
            Some("my%20repo")
        );
    }

    #[test]
    fn repo_segment_for_should_be_none_off_the_repo_shell() {
        assert_eq!(repo_segment_for("/", &PathBuf::from("index.html")), None);
        assert_eq!(
            repo_segment_for("/git-compose/bogus", &PathBuf::from("404.html")),
            None
        );
    }

    #[test]
    fn repo_head_links_should_always_include_the_two_feed_links() {
        let links = repo_head_links("git-compose", None);
        assert!(links.contains(
            "<link rel=\"alternate\" type=\"application/atom+xml\" title=\"Recent commits\" href=\"/api/v1/repos/git-compose/feed.atom\">"
        ));
        assert!(links.contains(
            "<link rel=\"alternate\" type=\"application/atom+xml\" title=\"Recent commits (all refs)\" href=\"/api/v1/repos/git-compose/feed.atom?all=1\">"
        ));
        assert!(!links.contains("vcs-git"));
    }

    #[test]
    fn repo_head_links_should_include_vcs_git_only_when_a_clone_base_is_configured() {
        let links = repo_head_links("git-compose", Some("https://git.example.net"));
        assert!(links.contains(
            "<link rel=\"vcs-git\" title=\"Git repository\" href=\"https://git.example.net/git-compose.git\">"
        ));
    }

    #[test]
    fn repo_head_links_should_trim_a_trailing_slash_off_the_clone_base() {
        let links = repo_head_links("git-compose", Some("https://git.example.net/"));
        assert!(links.contains("href=\"https://git.example.net/git-compose.git\""));
    }

    #[test]
    fn repo_head_links_should_escape_the_segment() {
        // A raw `"` should never reach here in practice (the URI is
        // percent-encoded before axum hands it over), but the escaping is
        // defense-in-depth against the attribute it's interpolated into.
        let links = repo_head_links(r#"weird"repo"#, Some("https://git.example.net"));
        assert!(!links.contains(r#""weird"repo""#));
        assert!(links.contains("weird&quot;repo"));
    }

    #[test]
    fn inject_repo_head_links_should_insert_before_head_close() {
        let body =
            b"<!doctype html><html><head><title>x</title></head><body></body></html>".to_vec();
        let injected = inject_repo_head_links(body, "git-compose", None);
        let injected = String::from_utf8(injected).unwrap();
        assert!(injected.contains("<title>x</title><link rel=\"alternate\""));
        assert!(injected.contains("feed.atom?all=1\"></head>"));
    }

    #[test]
    fn inject_repo_head_links_should_leave_a_headless_document_unchanged() {
        let body = b"<!doctype html><html><body>no head</body></html>".to_vec();
        let injected = inject_repo_head_links(body.clone(), "git-compose", None);
        assert_eq!(injected, body);
    }
}
