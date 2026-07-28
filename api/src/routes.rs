use axum::Router;
use axum::extract::Query;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use serde::Deserialize;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

use crate::error::ApiError;
use crate::handlers::{self, repos};
use crate::state::AppState;

pub fn build_router(state: AppState) -> Router {
    let api = Router::new()
        .route("/repos", get(repos::list_repos))
        .route("/repos/{repo}", get(handlers::not_implemented))
        .route("/repos/{repo}/refs", get(handlers::not_implemented))
        .route("/repos/{repo}/commits", get(handlers::not_implemented))
        .route(
            "/repos/{repo}/commits/{sha}",
            get(handlers::not_implemented),
        )
        .route(
            "/repos/{repo}/commits/{sha}/diff",
            get(handlers::not_implemented),
        )
        // ref/path boundary inside the wildcard is resolved by the handler
        // (branch names may contain `/`), so a single catch-all per view.
        .route("/repos/{repo}/tree/{*rest}", get(handlers::not_implemented))
        .route("/repos/{repo}/blob/{*rest}", get(handlers::not_implemented))
        .route("/repos/{repo}/raw/{*rest}", get(handlers::not_implemented))
        .route("/repos/{repo}/readme", get(handlers::not_implemented))
        .route(
            "/repos/{repo}/blame/{*rest}",
            get(handlers::not_implemented),
        )
        .route(
            "/repos/{repo}/archive/{*rest}",
            get(handlers::not_implemented),
        )
        .route("/repos/{repo}/feed.atom", get(handlers::not_implemented));

    // Smart HTTP lives outside /api/v1; `{repo_git}` is the directory name
    // including `.git` (axum cannot capture partial segments).
    let smart_http = Router::new()
        .route("/{repo_git}/info/refs", get(info_refs))
        .route(
            "/{repo_git}/git-upload-pack",
            post(handlers::not_implemented),
        )
        .route("/{repo_git}/git-receive-pack", any(receive_pack));

    let mut router = Router::new().nest("/api/v1", api).merge(smart_http);

    if let Some(static_dir) = &state.config.static_dir {
        router = router.fallback_service(ServeDir::new(static_dir));
    }

    router.layer(TraceLayer::new_for_http()).with_state(state)
}

#[derive(Deserialize)]
struct InfoRefsQuery {
    service: Option<String>,
}

/// Smart HTTP advertise stub. Push is SSH-only, so receive-pack is rejected
/// here and now even though upload-pack is not implemented yet.
async fn info_refs(Query(query): Query<InfoRefsQuery>) -> Response {
    if query.service.as_deref() == Some("git-receive-pack") {
        return ApiError::ReadOnly.into_response();
    }
    handlers::not_implemented().await
}

async fn receive_pack() -> ApiError {
    ApiError::ReadOnly
}
