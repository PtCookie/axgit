//! Handler for the Atom feed endpoint.
//!
//! The feed is a small, flat, fixed-structure document, so the XML is built
//! by hand with explicit escaping instead of pulling in a runtime XML crate
//! (docs/DECISIONS.md #12). Entry ids are `urn:sha1:` IRIs so they stay
//! stable no matter which host or proxy served the request.

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Response;

use super::{ATOM_CONTENT_TYPE, cached_response};
use crate::error::{ApiError, ErrorResponse};
use crate::repo::commits::{self, CommitInfo, CommitsPage};
use crate::repo::{RepoInfo, meta};
use crate::state::AppState;

const FEED_ENTRY_LIMIT: usize = 20;

/// Fallback for commits with unrepresentable timestamps — Atom requires
/// `<updated>` on every feed and entry.
const EPOCH: &str = "1970-01-01T00:00:00Z";

/// Atom feed of recent commits
///
/// The 20 most recent commits on HEAD. Absolute URLs are reconstructed from
/// `X-Forwarded-Proto`/`X-Forwarded-Host`, falling back to `Host`; entry ids
/// are `urn:sha1:{sha}` so they stay stable across hosts. An empty repository
/// returns `200` with no entries.
#[utoipa::path(
    get,
    path = "/api/v1/repos/{repo}/feed.atom",
    tag = "repos",
    params(
        ("repo" = String, Path, description = "Repository name without the `.git` suffix", example = "git-compose"),
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
        (status = 404, description = "`repo_not_found`", body = ErrorResponse),
    ),
)]
pub async fn get_feed(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let base = base_url(&headers);
    // The rendered XML embeds the base URL, so it is part of the cache key.
    let params = format!("base={base}");
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
            // Unborn HEAD (empty repository) serves an entry-less feed, not 404 —
            // the repo exists and feed readers keep polling it.
            let page = match repo.head().ok().and_then(|head| head.target()) {
                Some(oid) => commits::log(
                    repo,
                    oid,
                    &commits::LogParams {
                        path: None,
                        skip: 0,
                        limit: FEED_ENTRY_LIMIT,
                        include_body: false,
                        follow: false,
                        include_stat: false,
                    },
                )?,
                None => CommitsPage {
                    commits: Vec::new(),
                    next_cursor: None,
                },
            };
            let xml = render_feed(&base, &feed_name, &info, &page.commits);
            Ok((false, xml.into_bytes()))
        },
    )
    .await
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

fn render_feed(base: &str, name: &str, info: &RepoInfo, commits: &[CommitInfo]) -> String {
    // `<id>`/`rel="self"` must be this document's own URL, so they stay on
    // the API route even though entries' `rel="alternate"` points at the web
    // UI (see render_entry).
    let self_url = format!("{base}/api/v1/repos/{}/feed.atom", encode_segment(name));
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
        let xml = render_feed("http://localhost", "empty", &info, &[]);
        assert!(xml.contains("<feed xmlns=\"http://www.w3.org/2005/Atom\">"));
        assert!(xml.contains(&format!("<updated>{EPOCH}</updated>")));
        assert!(!xml.contains("<entry>"));
        assert!(!xml.contains("<subtitle>"));
    }
}
