//! Caching layer (docs/ARCHITECTURE.md#caching).
//!
//! Two caches live here. [`ScanCache`] holds the repository scan snapshot —
//! a single value, which does not fit moka's per-key model, so a `RwLock`
//! TTL is sufficient. [`ResponseCache`] is the moka LRU for serialized
//! per-repo responses, keyed by `(repo, endpoint, params)` and invalidated
//! by the HEAD sha + agefile mtime validator ([`crate::repo::meta::Validator`]).

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Bytes;
use tokio::sync::RwLock;

use crate::error::ApiError;
use crate::repo::meta::Validator;
use crate::repo::{RepoInfo, scan};

/// Bodies above this size are served but never cached, so one huge diff
/// cannot evict the rest of the cache. Matches the blob content limit.
pub const MAX_CACHEABLE_BODY: usize = 1 << 20;

/// Response cache key. `endpoint` is a static tag (`"summary"`, `"commits"`,
/// ...); `params` is the endpoint-specific normalized parameter string.
#[derive(Clone, Hash, PartialEq, Eq, Debug)]
pub struct ResponseKey {
    pub repo: String,
    pub endpoint: &'static str,
    pub params: String,
}

/// A cached serialized response. `validator == None` marks a sha-addressed
/// immutable entry, which is served without revalidation; otherwise the entry
/// is only served while the repository still matches `validator`.
#[derive(Clone)]
pub struct CachedResponse {
    pub body: Bytes,
    pub content_type: &'static str,
    pub etag: Option<String>,
    pub validator: Option<Validator>,
}

pub type ResponseCache = moka::future::Cache<ResponseKey, CachedResponse>;

/// Builds the response cache: capacity is weighed in body bytes, and the TTL
/// bounds staleness for changes the validators cannot see (config edits,
/// pushes to non-HEAD refs without a post-receive hook).
pub fn build_response_cache(max_bytes: u64, ttl: Duration) -> ResponseCache {
    moka::future::Cache::builder()
        .max_capacity(max_bytes)
        .weigher(|key: &ResponseKey, entry: &CachedResponse| {
            let size = key.repo.len() + key.params.len() + entry.body.len() + 200;
            u32::try_from(size).unwrap_or(u32::MAX)
        })
        .time_to_live(ttl)
        .build()
}

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
