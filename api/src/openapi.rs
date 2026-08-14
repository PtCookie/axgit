//! OpenAPI 3.1 description of the HTTP surface (docs/DECISIONS.md #15).
//!
//! The paths are written out by hand rather than collected from the axum
//! router: `tree`/`blob`/`raw`/`blame`/`archive` are single `{*rest}` catch-all
//! routes whose ref/path boundary is resolved per request
//! (`repo::resolve::resolve_ref_path`), so auto-collection would document
//! `/tree/{rest}` instead of the `/tree/{ref}/{path}` contract in docs/API.md.
//!
//! The generated document is served at [`crate::routes::OPENAPI_JSON_PATH`],
//! browsable at [`crate::routes::SWAGGER_UI_PATH`], and committed to
//! `docs/openapi.json`; `api/tests/openapi_test.rs` fails when the two drift.

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Axgit API",
        version = "1.0.0",
        description = "\
Read-only JSON API over bare Git repositories, plus Smart HTTP (fetch/clone) \
and an Atom feed.

There are no mutation endpoints and no authentication: writes happen over SSH \
on the git-server container, never here.

This document describes shapes, parameters and status codes. The semantic \
rules it cannot express — truncation caps, longest-ref matching for the \
`{ref}`/`{path}` split, merge-commit log simplification, and the exact \
conditions under which a field is `null` — live in `docs/API.md`, which \
remains the normative contract.",
        license(name = "MIT"),
    ),
    paths(
        crate::handlers::repos::list_repos,
        crate::handlers::repos::get_repo,
        crate::handlers::repos::get_refs,
        crate::handlers::tags::get_tag,
        crate::handlers::objects::get_object,
        crate::handlers::objects::get_object_raw,
        crate::handlers::commits::list_commits,
        crate::handlers::commits::get_commit,
        crate::handlers::commits::get_commit_diff,
        crate::handlers::diff::get_rev_diff,
        crate::handlers::diff::get_rawdiff,
        crate::handlers::diff::get_patch,
        crate::handlers::files::get_tree,
        crate::handlers::files::get_blob,
        crate::handlers::files::get_raw,
        crate::handlers::files::get_readme,
        crate::handlers::files::get_blame,
        crate::handlers::archive::get_archive,
        crate::handlers::feed::get_feed,
        crate::handlers::search::get_search,
        crate::handlers::stats::get_stats,
        crate::handlers::site::get_site,
        crate::handlers::site::get_site_logo,
        crate::handlers::site::get_site_favicon,
        crate::smart_http::info_refs,
        crate::smart_http::upload_pack,
        crate::routes::receive_pack,
    ),
    tags(
        (name = "repos", description = "Repository discovery, summary, refs, tag detail, feed, and commit statistics"),
        (name = "commits", description = "Commit log, detail, and structured diff"),
        (name = "diff", description = "Two-revision diff, raw unified diff, and format-patch output"),
        (name = "files", description = "Tree, blob, raw, readme, blame, and archive"),
        (name = "search", description = "Content, path, and commit-message search within a repository"),
        (name = "site", description = "Site-wide title, description, and readme — not tied to any one repository"),
        (name = "smart-http", description = "`git clone`/`fetch` over HTTP; push is rejected"),
    ),
)]
pub struct ApiDoc;
