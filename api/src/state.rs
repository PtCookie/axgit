use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Semaphore;

use crate::cache::{ResponseCache, ScanCache, build_response_cache};
use crate::config::Config;

/// Bounds concurrent bzip2/xz/zstd archive encoders (docs/DECISIONS.md #54).
/// Archives are never response-cached, so every request runs a fresh
/// encoder; an xz preset-6 encoder alone holds ~94 MiB, and an unbounded
/// number of concurrent ones can push the process past its container memory
/// limit. `tar.gz`/`zip` stream git's own output and don't take a permit.
const MAX_CONCURRENT_ARCHIVE_ENCODERS: usize = 4;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub scan_cache: Arc<ScanCache>,
    /// moka `Cache` is internally reference-counted; cloning is cheap.
    pub response_cache: ResponseCache,
    pub archive_encoder_limit: Arc<Semaphore>,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        let scan_cache = ScanCache::new(Duration::from_secs(config.cache_scan_ttl_secs));
        let response_cache = build_response_cache(
            config.cache_response_max_bytes,
            Duration::from_secs(config.cache_response_ttl_secs),
        );
        Self {
            config: Arc::new(config),
            scan_cache: Arc::new(scan_cache),
            response_cache,
            archive_encoder_limit: Arc::new(Semaphore::new(MAX_CONCURRENT_ARCHIVE_ENCODERS)),
        }
    }
}
