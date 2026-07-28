use std::sync::Arc;
use std::time::Duration;

use crate::cache::ScanCache;
use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub scan_cache: Arc<ScanCache>,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        let scan_cache = ScanCache::new(Duration::from_secs(config.cache_scan_ttl_secs));
        Self {
            config: Arc::new(config),
            scan_cache: Arc::new(scan_cache),
        }
    }
}
