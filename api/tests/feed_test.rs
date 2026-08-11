//! Integration tests for the Atom feed endpoint.

mod common;

use common::router_for;

use std::path::Path;

use axum::http::{StatusCode, header};

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

/// Extends [`setup_feed_repo`]'s three-commit `main` with:
/// - `dev`, forked from `main`'s first commit, carrying one commit dated
///   between main's second and third — interleaves with main under `all=1`'s
///   date-sorted walk.
/// - a lightweight tag `v1` on a commit reachable from *neither* `main` nor
///   `dev` (its branch is discarded after tagging, only pushed as the tag),
///   proving `all=1` walks `refs/tags/*` too, not just branches.
///
/// Returns (main shas oldest→newest, dev-only sha, tag-only sha).
fn setup_feed_repo_with_refs(root: &Path) -> (Vec<String>, String, String) {
    let main_shas = setup_feed_repo(root);
    let bare = root.join("feed.git");
    let work = tempfile::tempdir().unwrap();
    common::git(
        work.path(),
        &["clone", "--quiet", bare.to_str().unwrap(), "."],
    );

    common::git(
        work.path(),
        &["checkout", "--quiet", "-b", "dev", &main_shas[0]],
    );
    std::fs::write(work.path().join("dev.txt"), "dev\n").unwrap();
    common::git(work.path(), &["add", "."]);
    common::git_output(
        work.path(),
        &["commit", "--quiet", "-m", "dev work"],
        &[
            ("GIT_AUTHOR_DATE", "2026-07-02T18:00:00+09:00"),
            ("GIT_COMMITTER_DATE", "2026-07-02T18:00:00+09:00"),
        ],
    );
    let dev_sha = common::git_output(work.path(), &["rev-parse", "HEAD"], &[]);
    common::git(work.path(), &["push", "--quiet", "origin", "dev"]);

    common::git(
        work.path(),
        &["checkout", "--quiet", "-b", "tagged", &main_shas[0]],
    );
    std::fs::write(work.path().join("tagged.txt"), "tagged\n").unwrap();
    common::git(work.path(), &["add", "."]);
    common::git_output(
        work.path(),
        &["commit", "--quiet", "-m", "tag-only work"],
        &[
            ("GIT_AUTHOR_DATE", "2026-07-01T18:00:00+09:00"),
            ("GIT_COMMITTER_DATE", "2026-07-01T18:00:00+09:00"),
        ],
    );
    let tag_sha = common::git_output(work.path(), &["rev-parse", "HEAD"], &[]);
    common::git(work.path(), &["tag", "v1"]);
    common::git(work.path(), &["push", "--quiet", "origin", "refs/tags/v1"]);
    // The `tagged` branch itself is never pushed — `tag_sha` is reachable
    // only via `refs/tags/v1`.

    (main_shas, dev_sha, tag_sha)
}

/// `many.git` with 25 empty commits on `main` — enough to exceed the feed's
/// default 20-entry cap.
fn setup_many_commits_repo(root: &Path) {
    let bare = common::create_bare_repo(root, "many.git");
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
    // `<id>`/`rel="self"` stay on the API's own feed URL.
    assert!(xml.contains(
        "<link rel=\"self\" href=\"https://git.example.com/api/v1/repos/feed/feed.atom\"/>"
    ));
    // `rel="alternate"` points at the web UI's commit page, not the API.
    for line in xml
        .lines()
        .filter(|line| line.contains("rel=\"alternate\""))
    {
        assert!(
            line.starts_with(
                "    <link rel=\"alternate\" href=\"https://git.example.com/feed/commit/"
            ) && line.ends_with("\"/>"),
            "unexpected alternate link: {line}"
        );
    }
}

#[tokio::test]
async fn feed_percent_encodes_a_repo_name_with_reserved_characters() {
    let root = tempfile::tempdir().unwrap();
    common::create_bare_repo(root.path(), "my repo.git");
    common::commit_history(
        &root.path().join("my repo.git"),
        &[common::CommitSpec {
            file: "a.txt",
            content: "one\n",
            message: "first commit",
            date: "2026-07-01T12:00:00+09:00",
        }],
    );

    let (status, _, body) = common::get_bytes_with_headers(
        router_for(root.path()),
        "/api/v1/repos/my%20repo/feed.atom",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let xml = String::from_utf8(body).unwrap();
    assert!(xml.contains(
        "<link rel=\"self\" href=\"http://localhost/api/v1/repos/my%20repo/feed.atom\"/>"
    ));
    assert!(xml.contains("<link rel=\"alternate\" href=\"http://localhost/my%20repo/commit/"));
}

#[tokio::test]
async fn feed_caps_entries_at_limit() {
    let root = tempfile::tempdir().unwrap();
    setup_many_commits_repo(root.path());

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

    // `all=1` on a repo with no refs at all is the same entry-less carve-out,
    // not an error — there is nothing to push onto the revwalk.
    let (status, _, body) = common::get_bytes_with_headers(
        router_for(root.path()),
        "/api/v1/repos/empty/feed.atom?all=1",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
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

#[tokio::test]
async fn feed_ref_selects_a_branch() {
    let root = tempfile::tempdir().unwrap();
    let (_main_shas, dev_sha, _tag_sha) = setup_feed_repo_with_refs(root.path());

    let (status, _, body) = common::get_bytes_with_headers(
        router_for(root.path()),
        "/api/v1/repos/feed/feed.atom?ref=dev",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let xml = String::from_utf8(body).unwrap();
    assert!(xml.contains(&format!("<id>urn:sha1:{dev_sha}</id>")));

    // The default feed (HEAD = main) never sees dev's commit.
    let (_, _, default_body) =
        common::get_bytes_with_headers(router_for(root.path()), "/api/v1/repos/feed/feed.atom")
            .await;
    let default_xml = String::from_utf8(default_body).unwrap();
    assert!(!default_xml.contains(&dev_sha));
}

#[tokio::test]
async fn feed_all_includes_commits_from_every_branch_and_tag() {
    let root = tempfile::tempdir().unwrap();
    let (main_shas, dev_sha, tag_sha) = setup_feed_repo_with_refs(root.path());

    let (status, _, body) = common::get_bytes_with_headers(
        router_for(root.path()),
        "/api/v1/repos/feed/feed.atom?all=1",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let xml = String::from_utf8(body).unwrap();
    for sha in main_shas.iter().chain([&dev_sha, &tag_sha]) {
        assert!(
            xml.contains(&format!("<id>urn:sha1:{sha}</id>")),
            "missing {sha} from all=1 feed: {xml}"
        );
    }
}

#[tokio::test]
async fn feed_all_orders_entries_newest_first_across_branches() {
    let root = tempfile::tempdir().unwrap();
    let (main_shas, dev_sha, tag_sha) = setup_feed_repo_with_refs(root.path());

    let (_, _, body) = common::get_bytes_with_headers(
        router_for(root.path()),
        "/api/v1/repos/feed/feed.atom?all=1",
    )
    .await;
    let xml = String::from_utf8(body).unwrap();
    let position = |sha: &str| {
        xml.find(sha)
            .unwrap_or_else(|| panic!("sha {sha} missing from feed"))
    };
    // Dates: main[2] 07-03 15:30 > dev 07-02 18:00 > main[1] 07-02 12:00 >
    // tag 07-01 18:00 > main[0] 07-01 12:00. The default (unsorted) DFS walk
    // would drain one branch before the next; this only holds under
    // `Sort::TIME`.
    assert!(position(&main_shas[2]) < position(&dev_sha));
    assert!(position(&dev_sha) < position(&main_shas[1]));
    assert!(position(&main_shas[1]) < position(&tag_sha));
    assert!(position(&tag_sha) < position(&main_shas[0]));
}

#[tokio::test]
async fn feed_path_filters_entries() {
    let root = tempfile::tempdir().unwrap();
    let shas = setup_feed_repo(root.path());

    // b.txt was only touched by the second commit.
    let (status, _, body) = common::get_bytes_with_headers(
        router_for(root.path()),
        "/api/v1/repos/feed/feed.atom?path=b.txt",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let xml = String::from_utf8(body).unwrap();
    assert_eq!(assert_well_formed_and_count_entries(&xml), 1);
    assert!(xml.contains(&format!("<id>urn:sha1:{}</id>", shas[1])));

    // A path that never existed is an empty feed, not a 404.
    let (status, _, body) = common::get_bytes_with_headers(
        router_for(root.path()),
        "/api/v1/repos/feed/feed.atom?path=nope.txt",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let xml = String::from_utf8(body).unwrap();
    assert_eq!(assert_well_formed_and_count_entries(&xml), 0);
}

#[tokio::test]
async fn feed_limit_controls_entry_count() {
    let root = tempfile::tempdir().unwrap();
    setup_many_commits_repo(root.path());

    let (status, _, body) = common::get_bytes_with_headers(
        router_for(root.path()),
        "/api/v1/repos/many/feed.atom?limit=5",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let xml = String::from_utf8(body).unwrap();
    assert_eq!(assert_well_formed_and_count_entries(&xml), 5);
}

#[tokio::test]
async fn feed_rejects_invalid_params() {
    let root = tempfile::tempdir().unwrap();
    setup_feed_repo(root.path());

    for query in ["limit=0", "limit=101", "limit=abc", "all=yes"] {
        let (status, _, body) = common::get_bytes_with_headers(
            router_for(root.path()),
            &format!("/api/v1/repos/feed/feed.atom?{query}"),
        )
        .await;
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            (status, json["error"]["code"].as_str()),
            (StatusCode::BAD_REQUEST, Some("invalid_param")),
            "query {query} should be rejected: {json}"
        );
    }
}

#[tokio::test]
async fn feed_returns_404_for_unknown_ref() {
    let root = tempfile::tempdir().unwrap();
    setup_feed_repo(root.path());

    let (status, _, body) = common::get_bytes_with_headers(
        router_for(root.path()),
        "/api/v1/repos/feed/feed.atom?ref=nope",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["code"], "ref_not_found");
}

#[tokio::test]
async fn feed_ignores_ref_when_all_is_set() {
    let root = tempfile::tempdir().unwrap();
    setup_feed_repo(root.path());

    // A stale `ref` alongside `all=1` is never resolved — this would 404 if
    // it were.
    let (status, _, body) = common::get_bytes_with_headers(
        router_for(root.path()),
        "/api/v1/repos/feed/feed.atom?all=1&ref=nope",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let xml = String::from_utf8(body).unwrap();
    assert!(xml.contains("<id>http://localhost/api/v1/repos/feed/feed.atom?all=1</id>"));
    // `?ref=`/`&ref=` would show up in the id/self-link query if the stale
    // `ref` had leaked through — checked narrowly since `href=` itself
    // contains the substring "ref=".
    assert!(!xml.contains("?ref="));
    assert!(!xml.contains("&ref="));
    assert!(!xml.contains("&amp;ref="));
}

#[tokio::test]
async fn feed_self_link_carries_the_canonical_query() {
    let root = tempfile::tempdir().unwrap();
    setup_feed_repo_with_refs(root.path());

    let (status, _, body) = common::get_bytes_with_headers(
        router_for(root.path()),
        "/api/v1/repos/feed/feed.atom?limit=5&path=/b.txt/&ref=dev",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let xml = String::from_utf8(body).unwrap();
    let expected =
        "http://localhost/api/v1/repos/feed/feed.atom?ref=dev&amp;path=b.txt&amp;limit=5";
    assert!(xml.contains(&format!("<id>{expected}</id>")));
    assert!(xml.contains(&format!("<link rel=\"self\" href=\"{expected}\"/>")));

    // The default limit is omitted from the canonical query.
    let (_, _, body) = common::get_bytes_with_headers(
        router_for(root.path()),
        "/api/v1/repos/feed/feed.atom?limit=20",
    )
    .await;
    let xml = String::from_utf8(body).unwrap();
    assert!(xml.contains("<id>http://localhost/api/v1/repos/feed/feed.atom</id>"));
}

#[tokio::test]
async fn feed_uses_a_separate_cache_entry_for_each_param() {
    let root = tempfile::tempdir().unwrap();
    let (_main_shas, dev_sha, _tag_sha) = setup_feed_repo_with_refs(root.path());
    let router = router_for(root.path());

    let (_, _, default_body) =
        common::get_bytes_with_headers(router.clone(), "/api/v1/repos/feed/feed.atom").await;
    let default_xml = String::from_utf8(default_body).unwrap();
    assert_eq!(assert_well_formed_and_count_entries(&default_xml), 3);

    let (_, _, all_body) =
        common::get_bytes_with_headers(router.clone(), "/api/v1/repos/feed/feed.atom?all=1").await;
    let all_xml = String::from_utf8(all_body).unwrap();
    assert!(
        assert_well_formed_and_count_entries(&all_xml) > 3,
        "the all=1 response should not have reused the default cache entry: {all_xml}"
    );

    let (_, _, dev_body) =
        common::get_bytes_with_headers(router.clone(), "/api/v1/repos/feed/feed.atom?ref=dev")
            .await;
    let dev_xml = String::from_utf8(dev_body).unwrap();
    assert!(
        dev_xml.contains(&dev_sha),
        "the ref=dev response should not have reused the default cache entry: {dev_xml}"
    );

    let (_, _, limited_body) =
        common::get_bytes_with_headers(router, "/api/v1/repos/feed/feed.atom?limit=1").await;
    let limited_xml = String::from_utf8(limited_body).unwrap();
    assert_eq!(
        assert_well_formed_and_count_entries(&limited_xml),
        1,
        "the limit=1 response should not have reused the default cache entry: {limited_xml}"
    );
}
