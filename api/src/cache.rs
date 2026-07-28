//! Caching layer.
//!
//! Today this only holds the repository scan snapshot. The planned
//! `(repo, endpoint, params)`-keyed response cache with HEAD sha + agefile
//! mtime validators (docs/ARCHITECTURE.md#caching) will be added together
//! with `moka` once the first cacheable endpoint lands — the scan result is a
//! single snapshot, which does not fit moka's per-key model, so a `RwLock`
//! is sufficient here.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

use crate::error::ApiError;
use crate::repo::{RepoInfo, scan};

/// TTL cache for the repository scan (cgit `cache-scanrc-ttl` equivalent).
pub struct ScanCache {
    ttl: Duration,
    inner: RwLock<Option<Snapshot>>,
}

struct Snapshot {
    at: Instant,
    repos: Arc<Vec<RepoInfo>>,
}

impl ScanCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            inner: RwLock::new(None),
        }
    }

    /// Returns the cached scan, refreshing it when older than the TTL.
    pub async fn get_or_scan(&self, root: &Path) -> Result<Arc<Vec<RepoInfo>>, ApiError> {
        if let Some(snapshot) = self.inner.read().await.as_ref()
            && snapshot.at.elapsed() < self.ttl
        {
            return Ok(Arc::clone(&snapshot.repos));
        }

        let mut guard = self.inner.write().await;
        // Another request may have refreshed while we waited for the write lock.
        if let Some(snapshot) = guard.as_ref()
            && snapshot.at.elapsed() < self.ttl
        {
            return Ok(Arc::clone(&snapshot.repos));
        }

        let root = root.to_owned();
        let repos = tokio::task::spawn_blocking(move || scan::scan_repos(&root))
            .await
            .map_err(|err| ApiError::Internal(err.into()))??;
        let repos = Arc::new(repos);
        *guard = Some(Snapshot {
            at: Instant::now(),
            repos: Arc::clone(&repos),
        });
        Ok(repos)
    }
}
