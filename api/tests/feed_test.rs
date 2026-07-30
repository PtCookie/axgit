//! Integration tests for the Atom feed endpoint.

mod common;

use std::net::SocketAddr;
use std::path::Path;

use axum::Router;
use axum::http::{StatusCode, header};

use axgit::config::Config;
use axgit::routes::build_router;
use axgit::state::AppState;

fn router_for(repo_root: &Path) -> Router {
    let config = Config {
        repo_root: repo_root.to_owned(),
        static_dir: None,
        listen: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
        clone_url_base: None,
        cache_scan_ttl_secs: 60,
    };
    build_router(AppState::new(config))
}

/// `feed.git` with three commits (distinct dates, one XML-hostile message);
/// returns shas oldest → newest.
fn setup_feed_repo(root: &Path) -> Vec<String> {
    let bare = common::create_bare_repo(root, "feed.git");
    common::set_meta(&bare, "cgit", "desc", "Feed <fixture> & friends");
    common::commit_history(
        &bare,
        &[
            common::CommitSpec {
                file: "a.txt",
                content: "one\n",
                message: "first commit",
                date: "2026-07-01T12:00:00+09:00",
            },
            common::CommitSpec {
                file: "b.txt",
                content: "two\n",
                message: "fix: <b> & \"quotes\" 'x'",
                date: "2026-07-02T12:00:00+09:00",
            },
            common::CommitSpec {
                file: "c.txt",
                content: "three\n",
                message: "third commit",
                date: "2026-07-03T15:30:00+09:00",
            },
        ],
    )
}

/// Walks the whole document with quick-xml (panics on malformed XML) and
/// returns the number of `<entry>` elements.
fn assert_well_formed_and_count_entries(xml: &str) -> usize {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut entries = 0;
    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Eof) => break,
            Ok(quick_xml::events::Event::Start(start)) if start.name().as_ref() == b"entry" => {
                entries += 1;
            }
            Ok(_) => {}
            Err(err) => panic!("feed is not well-formed XML: {err}"),
        }
    }
    entries
}

#[tokio::test]
async fn feed_returns_escaped_atom_entries_newest_first() {
    let root = tempfile::tempdir().unwrap();
    let shas = setup_feed_repo(root.path());
    let (status, headers, body) =
        common::get_bytes_with_headers(router_for(root.path()), "/api/v1/repos/feed/feed.atom")
            .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers[header::CONTENT_TYPE],
        "application/atom+xml; charset=utf-8"
    );

    let xml = String::from_utf8(body).unwrap();
    assert_eq!(assert_well_formed_and_count_entries(&xml), 3);
    assert!(xml.contains("<feed xmlns=\"http://www.w3.org/2005/Atom\">"));
    assert!(xml.contains("<title>feed</title>"));
    // Repo description (cgit config) becomes the subtitle, escaped.
    assert!(xml.contains("<subtitle>Feed &lt;fixture&gt; &amp; friends</subtitle>"));
    // Feed <updated> is the newest commit's authordate.
    assert!(xml.contains("<updated>2026-07-03T15:30:00+09:00</updated>"));
    // Commit summary is escaped in the entry title.
    assert!(xml.contains("<title>fix: &lt;b&gt; &amp; &quot;quotes&quot; &apos;x&apos;</title>"));
    // Entry ids are host-independent urn IRIs.
    for sha in &shas {
        assert!(xml.contains(&format!("<id>urn:sha1:{sha}</id>")));
    }
    // Newest first: sha #2 appears before #1 before #0.
    let position = |sha: &str| {
        xml.find(sha)
            .unwrap_or_else(|| panic!("sha {sha} missing from feed"))
    };
    assert!(position(&shas[2]) < position(&shas[1]));
    assert!(position(&shas[1]) < position(&shas[0]));
    // Author name only — the raw email must never leak (DECISIONS #8).
    assert!(xml.contains("<author><name>Test Author</name></author>"));
    assert!(!xml.contains("author@example.com"));
}

#[tokio::test]
async fn feed_builds_base_url_from_forwarded_headers() {
    let root = tempfile::tempdir().unwrap();
    setup_feed_repo(root.path());

    // No Host header at all → deterministic localhost fallback.
    let (_, _, body) =
        common::get_bytes_with_headers(router_for(root.path()), "/api/v1/repos/feed/feed.atom")
            .await;
    let xml = String::from_utf8(body).unwrap();
    assert!(xml.contains("<id>http://localhost/api/v1/repos/feed/feed.atom</id>"));

    // Reverse-proxy headers win over Host.
    let (_, _, body) = common::get_bytes_with_request_headers(
        router_for(root.path()),
        "/api/v1/repos/feed/feed.atom",
        &[
            ("Host", "internal:8080"),
            ("X-Forwarded-Proto", "https"),
            ("X-Forwarded-Host", "git.example.com"),
        ],
    )
    .await;
    let xml = String::from_utf8(body).unwrap();
    assert!(xml.contains(
        "<link rel=\"self\" href=\"https://git.example.com/api/v1/repos/feed/feed.atom\"/>"
    ));
    assert!(xml.contains("href=\"https://git.example.com/api/v1/repos/feed/commits/"));
}

#[tokio::test]
async fn feed_caps_entries_at_limit() {
    let root = tempfile::tempdir().unwrap();
    let bare = common::create_bare_repo(root.path(), "many.git");
    let work = tempfile::tempdir().unwrap();
    common::git(
        work.path(),
        &["clone", "--quiet", bare.to_str().unwrap(), "."],
    );
    for i in 0..25 {
        common::git(
            work.path(),
            &[
                "commit",
                "--quiet",
                "--allow-empty",
                "-m",
                &format!("commit {i}"),
            ],
        );
    }
    common::git(work.path(), &["push", "--quiet", "origin", "HEAD:main"]);

    let (status, _, body) =
        common::get_bytes_with_headers(router_for(root.path()), "/api/v1/repos/many/feed.atom")
            .await;
    assert_eq!(status, StatusCode::OK);
    let xml = String::from_utf8(body).unwrap();
    assert_eq!(assert_well_formed_and_count_entries(&xml), 20);
}

#[tokio::test]
async fn feed_returns_entryless_document_for_empty_repo() {
    let root = tempfile::tempdir().unwrap();
    common::create_bare_repo(root.path(), "empty.git");

    let (status, headers, body) =
        common::get_bytes_with_headers(router_for(root.path()), "/api/v1/repos/empty/feed.atom")
            .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers[header::CONTENT_TYPE],
        "application/atom+xml; charset=utf-8"
    );
    let xml = String::from_utf8(body).unwrap();
    assert_eq!(assert_well_formed_and_count_entries(&xml), 0);
}

#[tokio::test]
async fn feed_returns_404_for_missing_repo() {
    let root = tempfile::tempdir().unwrap();
    let (status, _, body) =
        common::get_bytes_with_headers(router_for(root.path()), "/api/v1/repos/missing/feed.atom")
            .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["code"], "repo_not_found");
}
