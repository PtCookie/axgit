use anyhow::Context;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use axgit::config::Config;
use axgit::routes::build_router;
use axgit::state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Before `Config::load` on purpose: reading the config file warns about
    // unknown keys, and those warnings would go nowhere with no subscriber
    // installed. The subscriber setup reads no config of its own.
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = Config::load()?;

    let listener = TcpListener::bind(config.listen)
        .await
        .with_context(|| format!("failed to bind {}", config.listen))?;
    // Rendered before the event so the field is a plain path string rather
    // than `Some("…")` — this log line is JSON, and an operator grepping it
    // for which config file actually applied shouldn't have to read Rust
    // `Debug` output.
    let config_file = config
        .config
        .as_ref()
        .map(|path| path.display().to_string());
    tracing::info!(
        listen = %config.listen,
        repo_root = %config.repo_root.display(),
        config_file = config_file.as_deref().unwrap_or("none"),
        "starting axgit"
    );

    let router = build_router(AppState::new(config));
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    tokio::select! {
        () = ctrl_c => {}
        _ = terminate => {}
    }
}
