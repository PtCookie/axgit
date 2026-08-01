pub mod archive;
pub mod commits;
pub mod feed;
pub mod files;
pub mod repos;
pub mod search;

use axum::body::Bytes;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::cache::{CachedResponse, MAX_CACHEABLE_BODY, ResponseKey};
use crate::error::ApiError;
use crate::repo::meta::Validator;
use crate::repo::{meta, open};
use crate::state::AppState;

/// `Cache-Control` for responses addressed by a full commit sha (docs/API.md).
pub(crate) const IMMUTABLE_CACHE_CONTROL: &str = "public, max-age=31536000, immutable";

/// `Cache-Control` for validator-based responses: clients may store them but
/// must revalidate with `If-None-Match` (docs/API.md).
pub(crate) const NO_CACHE_CONTROL: &str = "no-cache";

pub(crate) const JSON_CONTENT_TYPE: &str = "application/json";
pub(crate) const ATOM_CONTENT_TYPE: &str = "application/atom+xml; charset=utf-8";

pub(crate) const DEFAULT_LIMIT: usize = 50;
pub(crate) const MAX_LIMIT: usize = 100;

/// Shared by `commits::list_commits` and `search::get_search` — both page
/// results with the same "default 50, 1-100, never clamped" rule
/// (docs/API.md). Parsed manually so an invalid value yields the JSON
/// `invalid_param` envelope instead of axum's plain-text 400.
pub(crate) fn parse_limit(raw: Option<&str>) -> Result<usize, ApiError> {
    let Some(raw) = raw else {
        return Ok(DEFAULT_LIMIT);
    };
    match raw.parse::<usize>() {
        Ok(limit) if (1..=MAX_LIMIT).contains(&limit) => Ok(limit),
        _ => Err(ApiError::InvalidParam(format!(
            "limit must be an integer between 1 and {MAX_LIMIT}"
        ))),
    }
}

/// Serves one per-repo endpoint through the response cache
/// (docs/ARCHITECTURE.md#caching). `compute` builds the serialized body and
/// reports whether the request addressed an immutable (full-sha) resource:
///
/// - immutable responses are cached without a validator and served with the
///   immutable `Cache-Control` (no `ETag`), never revalidated;
/// - all others carry a validator (HEAD sha + agefile mtime): a hit is served
///   only while the repository still matches, with an `ETag` + `no-cache`,
///   honoring `If-None-Match` with 304.
///
/// Concurrent misses may compute the same entry twice (no request coalescing;
/// moka `get_with` does not fit the validator flow) — acceptable at this scale.
/// Errors are never cached, and an error drops any stale entry for the key.
pub(crate) async fn cached_response<F>(
    state: &AppState,
    repo_name: &str,
    endpoint: &'static str,
    params: String,
    content_type: &'static str,
    request_headers: &HeaderMap,
    compute: F,
) -> Result<Response, ApiError>
where
    F: FnOnce(&git2::Repository) -> Result<(bool, Vec<u8>), ApiError> + Send + 'static,
{
    let key = ResponseKey {
        repo: repo_name.to_owned(),
        endpoint,
        params,
    };
    let entry = state.response_cache.get(&key).await;

    // Sha-addressed entries are immutable: served without touching the repo.
    if let Some(entry) = &entry
        && entry.validator.is_none()
    {
        return Ok(immutable_response(entry.content_type, entry.body.clone()));
    }

    enum Outcome {
        /// The cached entry still matches the repository state.
        Validated,
        Fresh {
            validator: Validator,
            immutable: bool,
            body: Vec<u8>,
        },
    }

    let root = state.config.repo_root.clone();
    let name = key.repo.clone();
    let expected = entry.as_ref().and_then(|entry| entry.validator);
    let outcome = tokio::task::spawn_blocking(move || {
        let repo = open::open_named(&root, &name)?;
        let current = meta::validator(&repo);
        if expected == Some(current) {
            return Ok(Outcome::Validated);
        }
        let (immutable, body) = compute(&repo)?;
        Ok(Outcome::Fresh {
            validator: current,
            immutable,
            body,
        })
    })
    .await
    .map_err(|err| ApiError::Internal(err.into()))?;

    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(err) => {
            // The repository is gone or no longer serves this request; drop
            // the entry so a stale body cannot be revived later.
            state.response_cache.invalidate(&key).await;
            return Err(err);
        }
    };

    match outcome {
        Outcome::Validated => {
            let entry = entry.expect("validated outcome requires a cache entry");
            let etag = entry
                .etag
                .as_deref()
                .expect("validated entries carry an etag");
            Ok(etag_response(
                request_headers,
                etag,
                entry.content_type,
                entry.body.clone(),
            ))
        }
        Outcome::Fresh {
            validator,
            immutable,
            body,
        } => {
            let body = Bytes::from(body);
            let (etag, cached) = if immutable {
                (
                    None,
                    CachedResponse {
                        body: body.clone(),
                        content_type,
                        etag: None,
                        validator: None,
                    },
                )
            } else {
                let etag = validator_etag(&validator);
                (
                    Some(etag.clone()),
                    CachedResponse {
                        body: body.clone(),
                        content_type,
                        etag: Some(etag),
                        validator: Some(validator),
                    },
                )
            };
            if body.len() <= MAX_CACHEABLE_BODY {
                state.response_cache.insert(key, cached).await;
            }
            Ok(match etag {
                None => immutable_response(content_type, body),
                Some(etag) => etag_response(request_headers, &etag, content_type, body),
            })
        }
    }
}

/// Strong `ETag` derived from the freshness validator; opaque to clients
/// (the format is not part of the API contract).
pub(crate) fn validator_etag(validator: &Validator) -> String {
    let head = validator
        .head
        .map_or_else(|| "unborn".to_owned(), |oid| oid.to_string());
    let mtime = validator
        .agefile_mtime
        .and_then(|mtime| mtime.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or_else(
            || "0".to_owned(),
            |age| format!("{}.{}", age.as_secs(), age.subsec_nanos()),
        );
    format!("\"{head}-{mtime}\"")
}

/// Strong `ETag` from the response body, for endpoints without a single
/// backing repository (the repos list).
pub(crate) fn body_etag(body: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("\"{:x}\"", Sha256::digest(body))
}

/// Whether `If-None-Match` matches `etag`, using the weak comparison RFC 9110
/// prescribes for 304 evaluation (`W/` prefixes ignored on both sides).
pub(crate) fn if_none_match(headers: &HeaderMap, etag: &str) -> bool {
    let own = etag.strip_prefix("W/").unwrap_or(etag);
    headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .any(|candidate| {
            candidate == "*" || candidate.strip_prefix("W/").unwrap_or(candidate) == own
        })
}

/// 304 response; RFC 9110 requires repeating the `ETag` (and caching headers)
/// the 200 response would have carried.
pub(crate) fn not_modified(etag: &str) -> Response {
    (
        StatusCode::NOT_MODIFIED,
        [
            (header::ETAG, header_value(etag)),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static(NO_CACHE_CONTROL),
            ),
        ],
    )
        .into_response()
}

fn immutable_response(content_type: &'static str, body: Bytes) -> Response {
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static(IMMUTABLE_CACHE_CONTROL),
            ),
        ],
        body,
    )
        .into_response()
}

/// 200 (or 304 on an `If-None-Match` match) with `ETag` + `no-cache`.
pub(crate) fn etag_response(
    request_headers: &HeaderMap,
    etag: &str,
    content_type: &'static str,
    body: Bytes,
) -> Response {
    if if_none_match(request_headers, etag) {
        return not_modified(etag);
    }
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
            (header::ETAG, header_value(etag)),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static(NO_CACHE_CONTROL),
            ),
        ],
        body,
    )
        .into_response()
}

fn header_value(value: &str) -> HeaderValue {
    HeaderValue::from_str(value).expect("generated header values are ASCII")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::IF_NONE_MATCH, value.parse().unwrap());
        headers
    }

    #[test]
    fn if_none_match_should_use_weak_comparison() {
        assert!(if_none_match(&headers_with("\"abc\""), "\"abc\""));
        assert!(if_none_match(&headers_with("W/\"abc\""), "\"abc\""));
        assert!(if_none_match(&headers_with("\"abc\""), "W/\"abc\""));
        assert!(!if_none_match(&headers_with("\"abc\""), "\"def\""));
    }

    #[test]
    fn if_none_match_should_handle_lists_and_star() {
        assert!(if_none_match(&headers_with("\"x\", \"abc\""), "\"abc\""));
        assert!(if_none_match(&headers_with("*"), "\"abc\""));
        assert!(!if_none_match(&HeaderMap::new(), "\"abc\""));
    }

    #[test]
    fn validator_etag_should_distinguish_states() {
        let unborn = Validator {
            head: None,
            agefile_mtime: None,
        };
        assert_eq!(validator_etag(&unborn), "\"unborn-0\"");

        let oid = git2::Oid::from_str("94739392266bcbd2a4dcc7e02f57b7cf4ba7ec02").unwrap();
        let with_head = Validator {
            head: Some(oid),
            agefile_mtime: Some(
                std::time::UNIX_EPOCH + std::time::Duration::new(1_753_000_000, 42),
            ),
        };
        assert_eq!(
            validator_etag(&with_head),
            "\"94739392266bcbd2a4dcc7e02f57b7cf4ba7ec02-1753000000.42\""
        );
        assert_ne!(validator_etag(&unborn), validator_etag(&with_head));
    }
}
