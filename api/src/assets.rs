//! Where the frontend static build is served from — a directory
//! (`AXGIT_STATIC_DIR`) or, by default, a copy of `web/dist` baked into the
//! binary at compile time (docs/DECISIONS.md #74, #88 — the single-binary
//! deploy path is the standard one). The opt-in `api-only` Cargo feature
//! drops the baked-in copy for a build with no bundled frontend at all;
//! `AXGIT_STATIC_DIR` still works either way. `shell.rs` reads page shells
//! through this module without needing to know which mode it's in;
//! `routes.rs` additionally uses [`serve_embedded_file`] directly to serve
//! real files (the embedded equivalent of `ServeDir`) when embedded.

use std::path::{Path, PathBuf};

use axum::extract::Request;
#[cfg(not(feature = "api-only"))]
use axum::http::HeaderMap;
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
#[cfg(not(feature = "api-only"))]
use axum::response::IntoResponse;
use axum::response::Response;
#[cfg(not(feature = "api-only"))]
use percent_encoding::percent_decode_str;

use crate::handlers::IMMUTABLE_CACHE_CONTROL;
#[cfg(not(feature = "api-only"))]
use crate::handlers::{if_none_match, not_modified};

#[cfg(not(feature = "api-only"))]
#[derive(rust_embed::Embed)]
#[folder = "../web/dist"]
struct WebDist;

/// Where the frontend build is read from. Cheap to clone (a `PathBuf` or a
/// unit variant), so it's threaded through router closures by value.
#[derive(Clone)]
pub enum Assets {
    Dir(PathBuf),
    #[cfg(not(feature = "api-only"))]
    Embedded,
}

impl Assets {
    /// `static_dir` (`AXGIT_STATIC_DIR`) always wins when set, so an
    /// operator can override a baked-in build without rebuilding. `None`
    /// means "serve no frontend" — only possible when built with the
    /// `api-only` feature and no directory was configured.
    pub fn resolve(static_dir: Option<&Path>) -> Option<Self> {
        if let Some(dir) = static_dir {
            return Some(Assets::Dir(dir.to_owned()));
        }
        #[cfg(not(feature = "api-only"))]
        {
            Some(Assets::Embedded)
        }
        #[cfg(feature = "api-only")]
        {
            None
        }
    }

    /// A short label for diagnostics (log lines only — never user-facing).
    fn label(&self) -> &'static str {
        match self {
            Assets::Dir(_) => "directory",
            #[cfg(not(feature = "api-only"))]
            Assets::Embedded => "embedded",
        }
    }

    /// Reads one file by its build-root-relative path (e.g.
    /// `__repo__/tree/index.html`) — used for page shells, which are a few
    /// KB, so an owned buffer is returned rather than threading a
    /// `Cow<'static, [u8]>` through `shell.rs` for a type that only exists
    /// in one of the two modes. `None` covers both "missing" and, for the
    /// embedded case, "not valid UTF-8 as a rust-embed key" (page shell
    /// paths are always ASCII, so this never actually happens in practice).
    pub async fn read(&self, relative: &Path) -> Option<Vec<u8>> {
        match self {
            Assets::Dir(dir) => tokio::fs::read(dir.join(relative)).await.ok(),
            #[cfg(not(feature = "api-only"))]
            Assets::Embedded => {
                let key = relative.to_str()?;
                WebDist::get(key).map(|file| file.data.into_owned())
            }
        }
    }

    /// Blocking counterpart to [`Self::read`], for the one-time startup read
    /// (`shell::ShellRoutes::load`) that runs before `build_router` hands
    /// back a `Router` for `axum::serve` to poll — there's no async runtime
    /// request-path to `.await` from yet, and a manifest this small blocks
    /// for a negligible time next to standing up a bound listener.
    pub fn read_sync(&self, relative: &Path) -> Option<Vec<u8>> {
        match self {
            Assets::Dir(dir) => std::fs::read(dir.join(relative)).ok(),
            #[cfg(not(feature = "api-only"))]
            Assets::Embedded => {
                let key = relative.to_str()?;
                WebDist::get(key).map(|file| file.data.into_owned())
            }
        }
    }
}

/// Prefix Astro's default `build.assets` config gives every content-hashed
/// static asset (`_astro/App.C3GteLlS.js`, …) — nothing else under
/// `web/dist` carries a hash in its filename (`404.html`, `favicon.svg`,
/// `robots.txt`, page shells), so a plain prefix match is exact, not a
/// heuristic.
const HASHED_ASSET_PREFIX: &str = "/_astro/";

fn is_hashed_asset_path(uri_path: &str) -> bool {
    uri_path.starts_with(HASHED_ASSET_PREFIX)
}

/// Marks content-hashed `_astro/*` responses immutable, mode-independent
/// (`Assets::Dir`'s `ServeDir` and [`serve_embedded_file`] both go through
/// this one layer rather than each setting the header themselves). Applied
/// only to a 200 or 304: a request for an asset that no longer exists in the
/// current build (e.g. a stale link from a previous deploy) falls through to
/// the 404 shell, which must never be cached as if it were immutable.
pub(crate) async fn immutable_cache_for_hashed_assets(request: Request, next: Next) -> Response {
    let is_hashed_asset = is_hashed_asset_path(request.uri().path());
    let mut response = next.run(request).await;
    if is_hashed_asset && matches!(response.status(), StatusCode::OK | StatusCode::NOT_MODIFIED) {
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static(IMMUTABLE_CACHE_CONTROL),
        );
    }
    response
}

pub(crate) fn log_missing_shell(assets: &Assets, relative: &Path) {
    tracing::error!(
        file = %relative.display(),
        source = assets.label(),
        "page shell missing from the static build",
    );
}

/// Serves a single file from the embedded `web/dist` copy — the embedded
/// mode's equivalent of `ServeDir`. `None` when there's no such file
/// (including a `..`-containing or otherwise unresolvable path — rust-embed's
/// own lookup already rejects those, since a generated key never contains
/// `..`), so the caller falls through to the page shell exactly as
/// `ServeDir::fallback(shell)` does for the directory mode.
///
/// No `Cache-Control` is set here directly — for a `_astro/*` path,
/// [`immutable_cache_for_hashed_assets`] adds it afterward as a
/// mode-independent layer shared with `Assets::Dir`'s `ServeDir` path
/// (docs/DECISIONS.md #77); every other embedded file (page shells aren't
/// served by this function, but e.g. `favicon.svg`/`robots.txt` are) is left
/// without one, matching `ServeDir`'s own default.
#[cfg(not(feature = "api-only"))]
pub fn serve_embedded_file(headers: &HeaderMap, uri_path: &str) -> Option<Response> {
    let decoded = percent_decode_str(uri_path.trim_start_matches('/'))
        .decode_utf8()
        .ok()?;
    let file = WebDist::get(&decoded)?;
    let etag = format!("\"{}\"", hex::encode(file.metadata.sha256_hash()));
    if if_none_match(headers, &etag) {
        return Some(not_modified(&etag));
    }
    let content_type = file.metadata.mimetype().to_owned();
    Some(
        (
            [(header::CONTENT_TYPE, content_type), (header::ETAG, etag)],
            file.data.into_owned(),
        )
            .into_response(),
    )
}

#[cfg(not(feature = "api-only"))]
mod hex {
    /// Lowercase hex encoding for a sha256 digest — the codebase's other
    /// sha256 `ETag` (`handlers/mod.rs::body_etag`) gets this for free from
    /// `sha2`'s `{:x}` `Digest` formatting, which isn't available for a
    /// plain `[u8; 32]`.
    pub(super) fn encode(bytes: [u8; 32]) -> String {
        use std::fmt::Write;
        let mut out = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            write!(out, "{byte:02x}").expect("writing to a String never fails");
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_should_prefer_a_configured_directory() {
        let dir = Path::new("/some/dir");
        assert!(matches!(Assets::resolve(Some(dir)), Some(Assets::Dir(d)) if d == dir));
    }

    #[cfg(feature = "api-only")]
    #[test]
    fn resolve_should_be_none_when_unconfigured_and_not_embedded() {
        assert!(Assets::resolve(None).is_none());
    }

    #[cfg(not(feature = "api-only"))]
    #[test]
    fn resolve_should_fall_back_to_embedded_when_unconfigured() {
        assert!(matches!(Assets::resolve(None), Some(Assets::Embedded)));
    }

    #[tokio::test]
    async fn dir_read_should_return_none_for_a_missing_file() {
        let dir = tempfile::tempdir().expect("failed to create tempdir");
        let assets = Assets::Dir(dir.path().to_owned());
        assert!(assets.read(Path::new("nope.html")).await.is_none());
    }

    #[tokio::test]
    async fn dir_read_should_read_an_existing_file() {
        let dir = tempfile::tempdir().expect("failed to create tempdir");
        std::fs::write(dir.path().join("index.html"), b"<html></html>").unwrap();
        let assets = Assets::Dir(dir.path().to_owned());
        assert_eq!(
            assets.read(Path::new("index.html")).await,
            Some(b"<html></html>".to_vec())
        );
    }

    #[cfg(not(feature = "api-only"))]
    #[test]
    fn hex_encode_should_match_a_known_digest() {
        assert_eq!(hex::encode([0u8; 32]), "0".repeat(64));
        assert_eq!(hex::encode([0xab; 32]), "ab".repeat(32));
    }
}
