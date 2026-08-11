//! Handler for the Atom feed endpoint.
//!
//! The feed is a small, flat, fixed-structure document, so the XML is built
//! by hand with explicit escaping instead of pulling in a runtime XML crate
//! (docs/DECISIONS.md #12). Entry ids are `urn:sha1:` IRIs so they stay
//! stable no matter which host or proxy served the request.

use std::path::Path as FsPath;

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use serde::Deserialize;
use utoipa::IntoParams;

use super::{ATOM_CONTENT_TYPE, cached_response, clean_path, parse_flag, parse_limit};
use crate::error::{ApiError, ErrorResponse};
use crate::repo::commits::{self, CommitInfo};
use crate::repo::{RepoInfo, meta, resolve};
use crate::state::AppState;

const FEED_DEFAULT_LIMIT: usize = 20;

/// Fallback for commits with unrepresentable timestamps — Atom requires
/// `<updated>` on every feed and entry.
const EPOCH: &str = "1970-01-01T00:00:00Z";

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FeedQuery {
    /// Branch, tag, or commit sha; HEAD when absent. Ignored when `all=1`.
    #[serde(rename = "ref")]
    #[param(example = "main")]
    r#ref: Option<String>,
    /// Only commits that changed this file or directory. A path that never
    /// existed yields an entry-less feed rather than a 404. No `follow`
    /// support — unlike the commit log, the feed never tracks a path across
    /// renames (docs/DECISIONS.md #61).
    path: Option<String>,
    /// `1`/`true` walks every local branch and tag (`refs/heads/*` +
    /// `refs/tags/*`) instead of just the selected `ref`, newest first by
    /// committer date. Any other value is `400 invalid_param`.
    #[param(value_type = Option<bool>)]
    all: Option<String>,
    /// Number of entries, default 20. Parsed manually so an invalid value
    /// yields the JSON `invalid_param` envelope instead of axum's plain-text
    /// 400. Never clamped.
    #[param(value_type = Option<u32>, minimum = 1, maximum = 100, example = 20)]
    limit: Option<String>,
}

/// Atom feed of recent commits
///
/// The most recent commits on `ref` (HEAD by default), 20 entries by
/// default. `all=1` walks every branch and tag instead. Absolute URLs are
/// reconstructed from `X-Forwarded-Proto`/`X-Forwarded-Host`, falling back
/// to `Host`; entry ids are `urn:sha1:{sha}` so they stay stable across
/// hosts. An empty repository, or `all=1` on a repository with no refs,
/// returns `200` with no entries.
#[utoipa::path(
    get,
    path = "/api/v1/repos/{repo}/feed.atom",
    tag = "repos",
    params(
        ("repo" = String, Path, description = "Repository name without the `.git` suffix", example = "git-compose"),
        FeedQuery,
    ),
    responses(
        (status = 200, description = "Atom 1.0 feed",
            content_type = "application/atom+xml",
            body = String,
            headers(
                ("ETag" = String, description = "Validator-derived; opaque"),
                ("Cache-Control" = String, description = "`no-cache`"),
            ),
        ),
        (status = 304, description = "`If-None-Match` matched the current `ETag`"),
        (status = 400, description = "`invalid_param` — bad `all` or `limit`", body = ErrorResponse),
        (status = 404, description = "`repo_not_found`, `ref_not_found`", body = ErrorResponse),
    ),
)]
pub async fn get_feed(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<FeedQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let all = parse_flag(query.all.as_deref(), "all")?;
    let limit = parse_limit(query.limit.as_deref(), FEED_DEFAULT_LIMIT)?;
    let path = clean_path(query.path.as_deref());
    // `all=1` walks every ref, so there is no ref to select — dropped here
    // so the cache key, the canonical query, and the walk all agree (the
    // same way `/commits`' `cursor` makes it ignore `ref`).
    let effective_ref = if all { None } else { query.r#ref.clone() };
    let feed_query = canonical_query(all, effective_ref.as_deref(), path.as_deref(), limit);

    let base = base_url(&headers);
    // The rendered XML embeds the base URL and the canonical query, so both
    // are part of the cache key — as is every param that changes the walk.
    let params = format!("base={base}&all={all}&ref={effective_ref:?}&path={path:?}&limit={limit}");
    let feed_name = name.clone();
    cached_response(
        &state,
        &name,
        "feed",
        params,
        ATOM_CONTENT_TYPE,
        &headers,
        move |repo| {
            let info = meta::read_repo_info(repo, &feed_name);
            let log_params = commits::LogParams {
                path: path.as_deref(),
                skip: 0,
                limit,
                include_body: false,
                follow: false,
                include_stat: false,
            };
            let commits = if all {
                commits::log_all_refs(repo, &log_params)?
            } else {
                // Unborn HEAD (empty repository) serves an entry-less feed,
                // not 404 — the repo exists and feed readers keep polling it.
                let start = match &effective_ref {
                    Some(refname) => Some(resolve::resolve_commit(repo, refname)?.id()),
                    None => repo.head().ok().and_then(|head| head.target()),
                };
                match start {
                    Some(oid) => commits::log(repo, oid, &log_params)?.commits,
                    None => Vec::new(),
                }
            };
            let xml = render_feed(&base, &feed_name, &feed_query, &info, &commits);
            Ok((false, xml.into_bytes()))
        },
    )
    .await
}

/// The canonical query string for this feed's `<id>`/`rel="self"`. Built
/// from the *parsed* params, not the raw query string, so that
/// `?path=/src/`, `?path=src`, and `?limit=20&path=src` all name the same
/// feed (RFC 4287 §4.2.6 requires a feed id that is stable and unique per
/// feed). Fixed param order regardless of request order; defaults are
/// omitted so the all-defaults feed's id matches today's un-parameterized
/// one exactly.
fn canonical_query(all: bool, r#ref: Option<&str>, path: Option<&FsPath>, limit: usize) -> String {
    let mut parts = Vec::new();
    if all {
        parts.push("all=1".to_owned());
    } else if let Some(refname) = r#ref {
        parts.push(format!("ref={}", encode_segment(refname)));
    }
    if let Some(path) = path {
        parts.push(format!("path={}", encode_segment(&path.to_string_lossy())));
    }
    if limit != FEED_DEFAULT_LIMIT {
        parts.push(format!("limit={limit}"));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("?{}", parts.join("&"))
    }
}

/// Reconstructs the external base URL from proxy headers. TLS terminates at
/// the reverse proxy (docs/DECISIONS.md #10), so `X-Forwarded-Proto` /
/// `X-Forwarded-Host` are authoritative when present; `Host` covers direct
/// access, `http://localhost` is the deterministic last resort.
fn base_url(headers: &HeaderMap) -> String {
    let scheme = header_value(headers, "x-forwarded-proto").unwrap_or("http");
    let host = header_value(headers, "x-forwarded-host")
        .or_else(|| header_value(headers, "host"))
        .unwrap_or("localhost");
    format!("{scheme}://{host}")
}

/// First comma-separated element of the header, trimmed; `None` when absent,
/// non-ASCII, or empty.
fn header_value<'h>(headers: &'h HeaderMap, name: &str) -> Option<&'h str> {
    headers
        .get(name)?
        .to_str()
        .ok()?
        .split(',')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn render_feed(
    base: &str,
    name: &str,
    query: &str,
    info: &RepoInfo,
    commits: &[CommitInfo],
) -> String {
    // `<id>`/`rel="self"` must be this document's own URL, so they stay on
    // the API route even though entries' `rel="alternate"` points at the web
    // UI (see render_entry). `query` is the canonical query string built by
    // `canonical_query`, so distinct parameterizations get distinct feed ids
    // (RFC 4287 §4.2.6).
    let self_url = format!(
        "{base}/api/v1/repos/{}/feed.atom{query}",
        encode_segment(name)
    );
    let updated = commits
        .first()
        .and_then(|commit| commit.authored_at.as_deref())
        .unwrap_or(EPOCH);

    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    xml.push_str("<feed xmlns=\"http://www.w3.org/2005/Atom\">\n");
    xml.push_str(&format!("  <title>{}</title>\n", xml_escape(name)));
    if let Some(description) = &info.description {
        xml.push_str(&format!(
            "  <subtitle>{}</subtitle>\n",
            xml_escape(description)
        ));
    }
    xml.push_str(&format!("  <id>{}</id>\n", xml_escape(&self_url)));
    xml.push_str(&format!(
        "  <link rel=\"self\" href=\"{}\"/>\n",
        xml_escape(&self_url)
    ));
    xml.push_str(&format!("  <updated>{}</updated>\n", xml_escape(updated)));
    for commit in commits {
        render_entry(&mut xml, base, name, commit);
    }
    xml.push_str("</feed>\n");
    xml
}

fn render_entry(xml: &mut String, base: &str, name: &str, commit: &CommitInfo) {
    let title = commit.summary.as_deref().unwrap_or("(no message)");
    let updated = commit.authored_at.as_deref().unwrap_or(EPOCH);
    // The web UI's commit page, not the API — feed readers should land
    // somewhere a human can read (docs/API.md, previously "provisional").
    // The sha is hex, so it needs no encoding.
    let commit_url = format!("{base}/{}/commit/{}", encode_segment(name), commit.sha);

    xml.push_str("  <entry>\n");
    xml.push_str(&format!("    <title>{}</title>\n", xml_escape(title)));
    xml.push_str(&format!("    <id>urn:sha1:{}</id>\n", commit.sha));
    xml.push_str(&format!(
        "    <link rel=\"alternate\" href=\"{}\"/>\n",
        xml_escape(&commit_url)
    ));
    xml.push_str(&format!("    <updated>{}</updated>\n", xml_escape(updated)));
    // Author email is never exposed, not even hashed (docs/DECISIONS.md #8).
    xml.push_str(&format!(
        "    <author><name>{}</name></author>\n",
        xml_escape(&commit.author.name)
    ));
    xml.push_str("  </entry>\n");
}

/// Percent-encodes a repository name for use as a URL path segment.
/// `open_named` only rejects `/`, `\`, and a leading `.` (docs/API.md), so a
/// name containing a space or other reserved byte is otherwise reachable and
/// would produce a syntactically invalid URI if interpolated raw.
fn encode_segment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

fn xml_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            _ => escaped.push(c),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_escape_should_escape_all_five_specials() {
        assert_eq!(
            xml_escape(r#"<b> & "it's""#),
            "&lt;b&gt; &amp; &quot;it&apos;s&quot;"
        );
        assert_eq!(xml_escape("plain"), "plain");
    }

    #[test]
    fn encode_segment_should_percent_encode_reserved_bytes() {
        assert_eq!(encode_segment("git-compose"), "git-compose");
        assert_eq!(encode_segment("my repo"), "my%20repo");
        assert_eq!(encode_segment("a/b"), "a%2Fb");
        assert_eq!(encode_segment("café"), "caf%C3%A9");
    }

    #[test]
    fn base_url_should_prefer_forwarded_headers() {
        let mut headers = HeaderMap::new();
        assert_eq!(base_url(&headers), "http://localhost");

        headers.insert("host", "direct.example".parse().unwrap());
        assert_eq!(base_url(&headers), "http://direct.example");

        headers.insert("x-forwarded-host", "git.example.com".parse().unwrap());
        headers.insert("x-forwarded-proto", "https, http".parse().unwrap());
        assert_eq!(base_url(&headers), "https://git.example.com");
    }

    #[test]
    fn render_feed_should_emit_entryless_feed_for_empty_history() {
        let info = RepoInfo {
            name: "empty".to_owned(),
            section: None,
            owner: None,
            description: None,
            default_branch: None,
            last_modified: None,
        };
        let xml = render_feed("http://localhost", "empty", "", &info, &[]);
        assert!(xml.contains("<feed xmlns=\"http://www.w3.org/2005/Atom\">"));
        assert!(xml.contains(&format!("<updated>{EPOCH}</updated>")));
        assert!(!xml.contains("<entry>"));
        assert!(!xml.contains("<subtitle>"));
    }

    #[test]
    fn render_feed_should_carry_the_query_into_id_and_self_link() {
        let info = RepoInfo {
            name: "repo".to_owned(),
            section: None,
            owner: None,
            description: None,
            default_branch: None,
            last_modified: None,
        };
        let xml = render_feed("http://localhost", "repo", "?ref=dev&limit=5", &info, &[]);
        let expected_url = "http://localhost/api/v1/repos/repo/feed.atom?ref=dev&limit=5";
        assert!(xml.contains(&format!("<id>{}</id>", xml_escape(expected_url))));
        assert!(xml.contains(&format!(
            "<link rel=\"self\" href=\"{}\"/>",
            xml_escape(expected_url)
        )));
    }

    #[test]
    fn canonical_query_should_omit_defaults_and_fix_order() {
        assert_eq!(canonical_query(false, None, None, FEED_DEFAULT_LIMIT), "");
        assert_eq!(canonical_query(false, None, None, 5), "?limit=5");
        assert_eq!(
            canonical_query(false, Some("feature/x"), None, 5),
            "?ref=feature%2Fx&limit=5"
        );
        // `all=1` wins over `ref` — the same order the handler computes
        // `effective_ref` in.
        assert_eq!(
            canonical_query(
                true,
                Some("dev"),
                Some(FsPath::new("src")),
                FEED_DEFAULT_LIMIT
            ),
            "?all=1&path=src"
        );
        assert_eq!(
            canonical_query(false, None, Some(FsPath::new("a b&c")), FEED_DEFAULT_LIMIT),
            "?path=a%20b%26c"
        );
    }
}
