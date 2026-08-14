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

use crate::assets::{self, Assets};
use crate::cgit_compat;
use crate::escape::xml_escape;

/// The reserved `getStaticPaths` param the `/{repo}` shells are built under.
/// Keep in sync with `web/src/lib/shell.ts::REPO_SHELL_PARAM`.
pub const REPO_SHELL_PARAM: &str = "__repo__";

/// Site-wide `<head>` metadata injected into *every* shell — index, repo
/// pages, and 404 alike — unlike the repo-only `<link>`s below
/// (docs/DECISIONS.md #70, `AXGIT_ROOT_TITLE`/`AXGIT_ROOT_DESC`). Both
/// `None` by default, in which case nothing is injected and the shell's own
/// hardcoded title/description stand unchanged. The web side
/// (`window.__axgit.fillSiteChrome`, docs/DECISIONS.md #71) reads these
/// custom `<meta>`s client-side to fill in the header brand and the real
/// `<meta name="description">` — the same split `fillRepoShell` already
/// draws for the repository name, kept here rather than overwriting the
/// real `<title>`/`<meta name="description">` server-side, which would only
/// flash before that same script corrects it.
/// Extends the same site-wide injection with cgit's `logo`/`logo-link`/
/// `favicon` (docs/DECISIONS.md #81). `logo`/`favicon` are already-resolved
/// `href`s — either the operator's own URL or axgit's serving route for the
/// file form (`routes.rs`, `branding.rs::BrandingAsset::href`) — this module
/// never needs to know which. The logo travels as a `<meta>`, read by
/// `fillSiteChrome` the same way `axgit:site-title`/`axgit:site-desc` are:
/// it's header chrome, and the header is `transition:persist`ed so it's
/// filled once per document rather than needing a fresh injection on every
/// swap. The favicon instead gets a real `<link rel="icon">` injected
/// server-side (and the shell's own default links stripped) — unlike the
/// logo, a browser fetches favicon `<link>`s while parsing `<head>`, before
/// any script has a chance to run, so a client-side swap would always fetch
/// the default first.
#[derive(Clone, Default)]
pub struct SiteHead {
    pub title: Option<String>,
    pub description: Option<String>,
    pub logo: Option<String>,
    pub logo_link: Option<String>,
    pub favicon: Option<String>,
    /// The favicon's `<link rel="icon" type="…">` MIME, resolved separately
    /// from `favicon` (the href) — the file form's href is the fixed,
    /// extensionless `/api/v1/site/favicon` route, with nothing to guess a
    /// MIME from at this layer (`branding.rs::BrandingAsset::content_type`
    /// resolves it once, from the configured path or URL, before `href`
    /// throws that extension away).
    pub favicon_type: Option<&'static str>,
}

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

/// Static-serving fallback, cgit-compatibility redirects included: a
/// `.git`-suffixed or cgit-query-shaped path (`docs/DECISIONS.md #35`) gets a
/// permanent redirect to its axgit equivalent; everything else falls through
/// to [`serve_shell`] unchanged.
pub async fn serve_shell_or_redirect(
    assets: Assets,
    clone_url_base: Option<String>,
    site: SiteHead,
    uri: Uri,
) -> Response {
    match cgit_compat::redirect_for(&uri) {
        Some(location) => Redirect::permanent(&location).into_response(),
        None => serve_shell(assets, clone_url_base, site, uri).await,
    }
}

/// Serving fallback: a request with no matching file gets the page shell for
/// its route shape, or the 404 shell — with a real 404 — if there is none.
/// Every shell gets `site`'s `<meta>`s injected when configured
/// (docs/DECISIONS.md #70); a `__repo__` shell additionally gets
/// per-repository `<link>`s (docs/DECISIONS.md #63) — the shell itself is
/// prerendered once under the placeholder param and can't know the
/// repository name (or, for `site`, that any config exists at all) at build
/// time. Reads through [`Assets`] (docs/DECISIONS.md #74), so this behaves
/// identically whether the build is on disk or baked into the binary.
pub async fn serve_shell(
    assets: Assets,
    clone_url_base: Option<String>,
    site: SiteHead,
    uri: Uri,
) -> Response {
    let (relative, status) = shell_for(uri.path());

    match assets.read(&relative).await {
        Some(body) => {
            let mut extra = site_head_meta(
                site.title.as_deref(),
                site.description.as_deref(),
                site.logo.as_deref(),
                site.logo_link.as_deref(),
            );
            if let Some(segment) = repo_segment_for(uri.path(), &relative) {
                extra.push_str(&repo_head_links(segment, clone_url_base.as_deref()));
            }
            // Only a configured favicon touches the shell's own default
            // `<link rel="icon">`s — unconfigured deployments keep them
            // untouched, matching every other `site` field's "nothing
            // injected, nothing removed" default.
            let body = match site.favicon.as_deref() {
                Some(href) => {
                    let body = strip_default_icon_links(&body);
                    extra.push_str(&favicon_head_link(href, site.favicon_type));
                    body
                }
                None => body,
            };
            let body = inject_before_head_close(body, &extra);
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
        None => {
            assets::log_missing_shell(&assets, &relative);
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

/// Inserts `extra` (raw HTML) before the shell's `</head>`, or returns `body`
/// unchanged if it has none — or if `extra` is empty, the common case when
/// neither site metadata nor a repo shell applies, which skips the scan
/// entirely. Byte-oriented (rather than parsing the HTML) — the shell is a
/// small, controlled document axgit itself produced, matching this file's
/// and `feed.rs`'s "hand-build small fixed documents" stance
/// (docs/DECISIONS.md #12). Shared by [`site_head_meta`]'s output and
/// [`repo_head_links`]'s, concatenated into one insertion by `serve_shell`
/// rather than splicing twice.
fn inject_before_head_close(body: Vec<u8>, extra: &str) -> Vec<u8> {
    if extra.is_empty() {
        return body;
    }
    const HEAD_CLOSE: &[u8] = b"</head>";
    let Some(pos) = find_subslice(&body, HEAD_CLOSE) else {
        return body;
    };

    let mut out = Vec::with_capacity(body.len() + extra.len());
    out.extend_from_slice(&body[..pos]);
    out.extend_from_slice(extra.as_bytes());
    out.extend_from_slice(&body[pos..]);
    out
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Builds the `axgit:site-title`/`axgit:site-desc`/`axgit:logo`/
/// `axgit:logo-link` `<meta>`s (docs/DECISIONS.md #70, #81) — each omitted
/// when its config value is unset, so an unconfigured deployment injects
/// nothing at all. The favicon has no `<meta>` counterpart here: see
/// [`favicon_head_link`].
fn site_head_meta(
    title: Option<&str>,
    description: Option<&str>,
    logo: Option<&str>,
    logo_link: Option<&str>,
) -> String {
    let mut meta = String::new();
    if let Some(title) = title {
        meta.push_str(&format!(
            "<meta name=\"axgit:site-title\" content=\"{}\">",
            xml_escape(title)
        ));
    }
    if let Some(description) = description {
        meta.push_str(&format!(
            "<meta name=\"axgit:site-desc\" content=\"{}\">",
            xml_escape(description)
        ));
    }
    if let Some(logo) = logo {
        meta.push_str(&format!(
            "<meta name=\"axgit:logo\" content=\"{}\">",
            xml_escape(logo)
        ));
    }
    if let Some(logo_link) = logo_link {
        meta.push_str(&format!(
            "<meta name=\"axgit:logo-link\" content=\"{}\">",
            xml_escape(logo_link)
        ));
    }
    meta
}

/// The `<link rel="icon">` for a configured favicon. `mime` is resolved
/// ahead of time by the caller (`branding.rs::BrandingAsset::content_type`)
/// rather than guessed from `href` here — the file form's href is always the
/// fixed, extensionless `/api/v1/site/favicon` route, so there is no
/// extension left in it to guess from by the time it reaches this function.
/// `type=` is omitted when `mime` is `None`, letting the browser sniff, same
/// as a bare `<link rel="icon" href="…">` with no type ever does.
fn favicon_head_link(href: &str, mime: Option<&str>) -> String {
    let href = xml_escape(href);
    match mime {
        Some(mime) => format!("<link rel=\"icon\" type=\"{mime}\" href=\"{href}\">"),
        None => format!("<link rel=\"icon\" href=\"{href}\">"),
    }
}

/// Strips every default `<link rel="icon" …>` the shell itself ships with
/// (`Layout.astro`'s `/favicon.svg` + `/favicon.ico` pair) — applied only
/// when a favicon is actually configured, so the two never both end up in
/// the same document. Byte-oriented, matching [`inject_before_head_close`]'s
/// "small controlled document" reasoning (docs/DECISIONS.md #12): this scans
/// for `<link` tags containing `rel="icon"` and drops each one whole.
fn strip_default_icon_links(body: &[u8]) -> Vec<u8> {
    const TAG_START: &[u8] = b"<link";
    const REL_ICON: &[u8] = b"rel=\"icon\"";

    let mut out = Vec::with_capacity(body.len());
    let mut rest = body;
    while let Some(start) = find_subslice(rest, TAG_START) {
        out.extend_from_slice(&rest[..start]);
        let tag = &rest[start..];
        let Some(end) = tag.iter().position(|&b| b == b'>') else {
            // No closing `>` — not a well-formed tag; stop rewriting and
            // keep the remainder verbatim rather than risk mangling it.
            out.extend_from_slice(tag);
            rest = &[];
            break;
        };
        let (whole_tag, after) = tag.split_at(end + 1);
        if find_subslice(whole_tag, REL_ICON).is_none() {
            out.extend_from_slice(whole_tag);
        }
        rest = after;
    }
    out.extend_from_slice(rest);
    out
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
    fn inject_before_head_close_should_insert_before_head_close() {
        let body =
            b"<!doctype html><html><head><title>x</title></head><body></body></html>".to_vec();
        let links = repo_head_links("git-compose", None);
        let injected = inject_before_head_close(body, &links);
        let injected = String::from_utf8(injected).unwrap();
        assert!(injected.contains("<title>x</title><link rel=\"alternate\""));
        assert!(injected.contains("feed.atom?all=1\"></head>"));
    }

    #[test]
    fn inject_before_head_close_should_leave_a_headless_document_unchanged() {
        let body = b"<!doctype html><html><body>no head</body></html>".to_vec();
        let links = repo_head_links("git-compose", None);
        let injected = inject_before_head_close(body.clone(), &links);
        assert_eq!(injected, body);
    }

    #[test]
    fn inject_before_head_close_should_be_a_no_op_for_empty_extra() {
        let body = b"<!doctype html><html><head></head><body></body></html>".to_vec();
        let injected = inject_before_head_close(body.clone(), "");
        assert_eq!(injected, body);
    }

    #[test]
    fn site_head_meta_should_omit_all_four_when_unset() {
        assert_eq!(site_head_meta(None, None, None, None), "");
    }

    #[test]
    fn site_head_meta_should_include_only_the_configured_fields() {
        let title_only = site_head_meta(Some("PtCookie Git"), None, None, None);
        assert!(title_only.contains("<meta name=\"axgit:site-title\" content=\"PtCookie Git\">"));
        assert!(!title_only.contains("site-desc"));
        assert!(!title_only.contains("axgit:logo"));

        let desc_only = site_head_meta(None, Some("Self-hosted repositories"), None, None);
        assert!(!desc_only.contains("site-title"));
        assert!(
            desc_only
                .contains("<meta name=\"axgit:site-desc\" content=\"Self-hosted repositories\">")
        );

        let logo_only = site_head_meta(None, None, Some("/api/v1/site/logo"), None);
        assert!(logo_only.contains("<meta name=\"axgit:logo\" content=\"/api/v1/site/logo\">"));
        assert!(!logo_only.contains("logo-link"));

        let logo_link_only = site_head_meta(None, None, None, Some("https://example.net"));
        assert!(!logo_link_only.contains("axgit:logo\""));
        assert!(
            logo_link_only
                .contains("<meta name=\"axgit:logo-link\" content=\"https://example.net\">")
        );
    }

    #[test]
    fn site_head_meta_should_escape_all_fields() {
        let meta = site_head_meta(
            Some(r#"a"b"#),
            Some(r#"c"d"#),
            Some(r#"e"f"#),
            Some(r#"g"h"#),
        );
        assert!(!meta.contains(r#""a"b""#));
        assert!(meta.contains("a&quot;b"));
        assert!(meta.contains("c&quot;d"));
        assert!(meta.contains("e&quot;f"));
        assert!(meta.contains("g&quot;h"));
    }

    #[test]
    fn favicon_head_link_should_include_the_given_type() {
        let link = favicon_head_link("/api/v1/site/favicon", Some("image/svg+xml"));
        assert_eq!(
            link,
            "<link rel=\"icon\" type=\"image/svg+xml\" href=\"/api/v1/site/favicon\">"
        );
    }

    #[test]
    fn favicon_head_link_should_omit_type_when_none() {
        let link = favicon_head_link("https://example.net/icon", None);
        assert_eq!(
            link,
            "<link rel=\"icon\" href=\"https://example.net/icon\">"
        );
    }

    #[test]
    fn favicon_head_link_should_escape_the_href() {
        let link = favicon_head_link(r#"/x"y.svg"#, None);
        assert!(!link.contains(r#"href="/x"y.svg""#));
        assert!(link.contains("x&quot;y.svg"));
    }

    #[test]
    fn strip_default_icon_links_should_remove_only_icon_links() {
        let body = b"<head><link rel=\"icon\" type=\"image/svg+xml\" href=\"/favicon.svg\"><link rel=\"icon\" href=\"/favicon.ico\"><link rel=\"stylesheet\" href=\"/a.css\"></head>".to_vec();
        let stripped = strip_default_icon_links(&body);
        let stripped = String::from_utf8(stripped).unwrap();
        assert!(!stripped.contains("rel=\"icon\""));
        assert!(stripped.contains("<link rel=\"stylesheet\" href=\"/a.css\">"));
    }

    #[test]
    fn strip_default_icon_links_should_be_a_no_op_without_any_icon_link() {
        let body = b"<head><link rel=\"stylesheet\" href=\"/a.css\"></head>".to_vec();
        assert_eq!(strip_default_icon_links(&body), body);
    }
}
