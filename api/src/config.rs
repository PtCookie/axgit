use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;

/// Axgit — read-only web frontend for bare Git repositories.
#[derive(Parser, Debug, Clone)]
#[command(name = "axgit", version)]
pub struct Config {
    /// Directory containing bare repositories (`*.git`).
    #[arg(long, env = "AXGIT_REPO_ROOT", default_value = "/srv/git")]
    pub repo_root: PathBuf,

    /// Static frontend build (web/dist) to serve at `/`.
    #[arg(long, env = "AXGIT_STATIC_DIR")]
    pub static_dir: Option<PathBuf>,

    /// Socket address to listen on.
    #[arg(long, env = "AXGIT_LISTEN", default_value = "0.0.0.0:8080")]
    pub listen: SocketAddr,

    /// Base URL used when displaying clone URLs.
    #[arg(long, env = "AXGIT_CLONE_URL_BASE")]
    pub clone_url_base: Option<String>,

    /// Repository scan cache TTL in seconds.
    #[arg(long, env = "AXGIT_CACHE_SCAN_TTL", default_value_t = 60)]
    pub cache_scan_ttl_secs: u64,

    /// Response cache TTL in seconds. The cache is invalidated by HEAD/agefile
    /// validators; the TTL only bounds staleness for out-of-band changes the
    /// validators cannot see (e.g. a manual config edit).
    #[arg(long, env = "AXGIT_CACHE_RESPONSE_TTL", default_value_t = 300)]
    pub cache_response_ttl_secs: u64,

    /// Response cache capacity in bytes.
    #[arg(long, env = "AXGIT_CACHE_RESPONSE_MAX_BYTES", default_value_t = 32 * 1024 * 1024)]
    pub cache_response_max_bytes: u64,
}
