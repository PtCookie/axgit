use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::Uri;
use axum::routing::{MethodRouter, any, get, post};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use crate::error::{ApiError, ErrorResponse};
use crate::handlers::{archive, commits, diff, feed, files, repos, search, stats};
use crate::openapi::ApiDoc;
use crate::shell;
use crate::smart_http;
use crate::state::AppState;

/// Where the generated spec is served, and the file `docs/openapi.json` mirrors.
pub const OPENAPI_JSON_PATH: &str = "/api/v1/openapi.json";
/// Where Swagger UI is mounted (assets are vendored into the binary).
pub const SWAGGER_UI_PATH: &str = "/swagger-ui";

pub fn build_router(state: AppState) -> Router {
    let api = Router::new()
        .route("/repos", get(repos::list_repos))
        .route("/repos/{repo}", get(repos::get_repo))
        .route("/repos/{repo}/refs", get(repos::get_refs))
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

    if let Some(static_dir) = &state.config.static_dir {
        // The Astro build has one HTML file per route *shape*, not per
        // repository (docs/DECISIONS.md #17): serve real files first, then
        // map whatever is left onto the matching prerendered page shell —
        // or `404.html` with a real 404 status.
        let dir = static_dir.clone();
        let shell: MethodRouter<()> =
            get(move |uri: Uri| shell::serve_shell_or_redirect(dir.clone(), uri));
        router = router.fallback_service(ServeDir::new(static_dir).fallback(shell));
    }

    router.layer(TraceLayer::new_for_http()).with_state(state)
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
