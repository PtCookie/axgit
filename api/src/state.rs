use std::sync::Arc;
use std::time::Duration;

use crate::cache::{ResponseCache, ScanCache, build_response_cache};
use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub scan_cache: Arc<ScanCache>,
    /// moka `Cache` is internally reference-counted; cloning is cheap.
    pub response_cache: ResponseCache,
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
        }
    }
}
