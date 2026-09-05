use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
#[cfg(not(feature = "api-only"))]
use axum::http::HeaderMap;
use axum::http::Uri;
use axum::middleware;
use axum::routing::{MethodRouter, any, get, post};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use crate::assets;
use crate::assets::Assets;
use crate::branding::{BrandingAsset, sanitize_logo_link};
use crate::error::{ApiError, ErrorResponse};
use crate::handlers::{
    archive, commits, diff, feed, files, objects, repos, search, site, stats, tags,
};
use crate::openapi::ApiDoc;
use crate::shell;
use crate::smart_http;
use crate::state::AppState;

/// Where the generated spec is served, and the file `openapi.json` mirrors.
pub const OPENAPI_JSON_PATH: &str = "/api/v1/openapi.json";
/// Where Swagger UI is mounted (assets are vendored into the binary).
pub const SWAGGER_UI_PATH: &str = "/swagger-ui";

pub fn build_router(state: AppState) -> Router {
    let api = Router::new()
        .route("/repos", get(repos::list_repos))
        .route("/repos/{repo}", get(repos::get_repo))
        .route("/repos/{repo}/refs", get(repos::get_refs))
        // A tag name may itself contain `/` (e.g. `release/1.0`), like the
        // ref half of the `{*rest}` routes below — but unlike those, there is
        // no ref/path boundary to resolve: the whole remainder is the name.
        .route("/repos/{repo}/tags/{*name}", get(tags::get_tag))
        // The one by-oid entry point in this API (docs/API.md) — `{oid}` is
        // a fixed-length hex id with no `/`, so a plain segment (not a
        // wildcard) is enough, unlike every ref-addressed route above/below.
        .route("/repos/{repo}/objects/{oid}", get(objects::get_object))
        .route(
            "/repos/{repo}/objects/{oid}/raw",
            get(objects::get_object_raw),
        )
        .route("/repos/{repo}/commits", get(commits::list_commits))
        .route("/repos/{repo}/commits/{sha}", get(commits::get_commit))
        .route(
            "/repos/{repo}/commits/{sha}/diff",
            get(commits::get_commit_diff),
        )
        .route("/repos/{repo}/diff", get(diff::get_rev_diff))
        .route("/repos/{repo}/rawdiff", get(diff::get_rawdiff))
        .route("/repos/{repo}/patch", get(diff::get_patch))
        // ref/path boundary inside the wildcard is resolved per request by
        // longest-ref matching (branch names may contain `/`), so a single
        // catch-all per view — see `repo::resolve::resolve_ref_path`.
        .route("/repos/{repo}/tree/{*rest}", get(files::get_tree))
        .route("/repos/{repo}/blob/{*rest}", get(files::get_blob))
        .route("/repos/{repo}/raw/{*rest}", get(files::get_raw))
        .route("/repos/{repo}/readme", get(files::get_readme))
        .route("/repos/{repo}/blame/{*rest}", get(files::get_blame))
        .route("/repos/{repo}/archive/{*rest}", get(archive::get_archive))
        .route("/repos/{repo}/feed.atom", get(feed::get_feed))
        .route("/repos/{repo}/search", get(search::get_search))
        .route("/repos/{repo}/stats", get(stats::get_stats))
        .route("/site", get(site::get_site))
        .route("/site/logo", get(site::get_site_logo))
        .route("/site/favicon", get(site::get_site_favicon))
        // `nest`ed routers inherit the outer `fallback_service` (the SPA shell
        // below), so an unmatched `/api/v1/...` path must get its own JSON
        // 404 rather than falling through to `index.html`.
        .fallback(api_not_found);

    // Smart HTTP lives outside /api/v1; `{repo_git}` is the directory name
    // including `.git` (axum cannot capture partial segments).
    let smart_http = Router::new()
        .route("/{repo_git}/info/refs", get(smart_http::info_refs))
        .route(
            "/{repo_git}/git-upload-pack",
            post(smart_http::upload_pack)
                .route_layer(DefaultBodyLimit::max(smart_http::MAX_REQUEST_BODY)),
        )
        .route("/{repo_git}/git-receive-pack", any(receive_pack));

    // Swagger UI serves the spec at OPENAPI_JSON_PATH itself, so there is no
    // separate handler; the static fallback below only sees unmatched paths.
    let mut router = Router::new()
        .nest("/api/v1", api)
        .merge(smart_http)
        .merge(SwaggerUi::new(SWAGGER_UI_PATH).url(OPENAPI_JSON_PATH, ApiDoc::openapi()));

    // `Assets::resolve` prefers `AXGIT_STATIC_DIR` when set, else falls back
    // to the binary's embedded copy of `web/dist` — the default build
    // (docs/DECISIONS.md #74, #88). `None` (neither configured nor embedded)
    // serves no frontend at all, which only happens with the opt-in
    // `api-only` Cargo feature.
    if let Some(assets) = Assets::resolve(state.config.static_dir.as_deref()) {
        // Read once at startup (docs/DECISIONS.md #88) — the table never
        // changes without a restart, so every request reuses this `Arc`
        // rather than re-reading and re-parsing `shell-routes.json`.
        let routes = Arc::new(shell::ShellRoutes::load(&assets));
        // `clone_url_base` rides along so a `__repo__` shell can get its
        // `rel="vcs-git"` link injected (docs/DECISIONS.md #63); `site`
        // similarly carries `root_title`/`root_desc` into every shell's
        // injected `<meta>`s (docs/DECISIONS.md #70).
        let clone_url_base = state.config.clone_url_base.clone();
        // `logo`/`favicon` resolve to the `href` the shell should emit — the
        // configured URL verbatim, or axgit's own serving route when it's a
        // local file (docs/DECISIONS.md #81); `logo_link` is validated
        // separately since an unsafe value there degrades to unset rather
        // than failing the whole logo. `favicon_type` is resolved from the
        // *parsed* asset (its real extension or URL), not from `favicon`
        // itself — the local-file href has none to guess from.
        let favicon_asset = state.config.favicon.as_deref().map(BrandingAsset::parse);
        let site = shell::SiteHead {
            title: state.config.root_title.clone(),
            description: state.config.root_desc.clone(),
            logo: state
                .config
                .logo
                .as_deref()
                .map(|raw| BrandingAsset::parse(raw).href("/api/v1/site/logo")),
            logo_link: state
                .config
                .logo_link
                .as_deref()
                .and_then(sanitize_logo_link),
            favicon: favicon_asset
                .as_ref()
                .map(|asset| asset.href("/api/v1/site/favicon")),
            favicon_type: favicon_asset.as_ref().and_then(BrandingAsset::content_type),
        };

        router = match assets {
            Assets::Dir(dir) => {
                // The Astro build has one HTML file per route *shape*, not
                // per repository (docs/DECISIONS.md #17): `ServeDir` serves
                // real files first, then maps whatever is left onto the
                // matching prerendered page shell — or `404.html` with a
                // real 404 status.
                let shell_assets = Assets::Dir(dir.clone());
                let routes = routes.clone();
                let shell: MethodRouter<()> = get(move |uri: Uri| {
                    shell::serve_shell_or_redirect(
                        shell_assets.clone(),
                        routes.clone(),
                        clone_url_base.clone(),
                        site.clone(),
                        uri,
                    )
                });
                // `append_index_html_on_directories` is off: with it on,
                // `ServeDir` would serve `dir/index.html` for `/` itself
                // directly, bypassing `fallback(shell)` (and so `site`'s
                // injected `<meta>`s) entirely — the only request shape in
                // this build that maps onto a real directory. Every other
                // route shape (e.g. `/git-compose`) has no matching
                // directory at all, so it already fell through to the shell
                // regardless of this flag; this only changes `/` itself.
                router.fallback_service(
                    ServeDir::new(dir)
                        .append_index_html_on_directories(false)
                        .fallback(shell),
                )
            }
            // The embedded mode's own "serve a real file, else fall through
            // to the shell" ordering, mirroring `ServeDir(...).fallback(shell)`
            // above without depending on the filesystem.
            #[cfg(not(feature = "api-only"))]
            Assets::Embedded => {
                let embedded: MethodRouter<()> =
                    get(move |uri: Uri, headers: HeaderMap| async move {
                        match assets::serve_embedded_file(&headers, uri.path()) {
                            Some(response) => response,
                            None => {
                                shell::serve_shell_or_redirect(
                                    Assets::Embedded,
                                    routes.clone(),
                                    clone_url_base.clone(),
                                    site.clone(),
                                    uri,
                                )
                                .await
                            }
                        }
                    });
                router.fallback_service(embedded)
            }
        };
    }

    // Mode-independent: content-hashed `_astro/*` assets get an immutable
    // `Cache-Control` whether they came from `Assets::Dir`'s `ServeDir` or
    // `Assets::Embedded`'s `serve_embedded_file` above — one layer here
    // instead of duplicating the header logic in both serving paths
    // (docs/DECISIONS.md #77). A no-op when neither mode is configured
    // (no `_astro/*` request ever reaches a 200/304 to mark).
    router
        .layer(middleware::from_fn(
            assets::immutable_cache_for_hashed_assets,
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Push is SSH-only (CLAUDE.md invariant): every method on this path is 403.
#[utoipa::path(
    post,
    path = "/{repo_git}/git-receive-pack",
    tag = "smart-http",
    params(
        ("repo_git" = String, Path, description = "Repository directory name **including** the `.git` suffix", example = "git-compose.git"),
    ),
    responses(
        (status = 403, description = "`read_only` — always; push via SSH instead. Every HTTP method answers the same way.", body = ErrorResponse),
    ),
)]
pub(crate) async fn receive_pack() -> ApiError {
    ApiError::ReadOnly
}

/// Catches unmatched `/api/v1/...` paths so they answer with the standard
/// JSON error envelope instead of inheriting the outer SPA-shell fallback.
async fn api_not_found() -> ApiError {
    ApiError::NotFound
}
