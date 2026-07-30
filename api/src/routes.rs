use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{any, get, post};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use crate::error::{ApiError, ErrorResponse};
use crate::handlers::{archive, commits, feed, files, repos};
use crate::openapi::ApiDoc;
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
        // ref/path boundary inside the wildcard is resolved per request by
        // longest-ref matching (branch names may contain `/`), so a single
        // catch-all per view — see `repo::resolve::resolve_ref_path`.
        .route("/repos/{repo}/tree/{*rest}", get(files::get_tree))
        .route("/repos/{repo}/blob/{*rest}", get(files::get_blob))
        .route("/repos/{repo}/raw/{*rest}", get(files::get_raw))
        .route("/repos/{repo}/readme", get(files::get_readme))
        .route("/repos/{repo}/blame/{*rest}", get(files::get_blame))
        .route("/repos/{repo}/archive/{*rest}", get(archive::get_archive))
        .route("/repos/{repo}/feed.atom", get(feed::get_feed));

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
        router = router.fallback_service(ServeDir::new(static_dir));
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
