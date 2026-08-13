//! Where the frontend static build is served from — a directory
//! (`AXGIT_STATIC_DIR`) or, behind the opt-in `embed-web` Cargo feature, a
//! copy of `web/dist` baked into the binary at compile time
//! (docs/DECISIONS.md #74, the single-binary deploy path). `shell.rs` reads
//! page shells through this module without needing to know which mode it's
//! in; `routes.rs` additionally uses [`serve_embedded_file`] directly to
//! serve real files (the embedded equivalent of `ServeDir`) when embedded.

use std::path::{Path, PathBuf};

#[cfg(feature = "embed-web")]
use axum::http::{HeaderMap, header};
#[cfg(feature = "embed-web")]
use axum::response::{IntoResponse, Response};
#[cfg(feature = "embed-web")]
use percent_encoding::percent_decode_str;

#[cfg(feature = "embed-web")]
use crate::handlers::{if_none_match, not_modified};

#[cfg(feature = "embed-web")]
#[derive(rust_embed::Embed)]
#[folder = "../web/dist"]
struct WebDist;

/// Where the frontend build is read from. Cheap to clone (a `PathBuf` or a
/// unit variant), so it's threaded through router closures by value.
#[derive(Clone)]
pub enum Assets {
    Dir(PathBuf),
    #[cfg(feature = "embed-web")]
    Embedded,
}

impl Assets {
    /// `static_dir` (`AXGIT_STATIC_DIR`) always wins when set, so an
    /// operator can override a baked-in build without rebuilding. `None`
    /// means "serve no frontend" — only possible when the `embed-web`
    /// feature is off and no directory was configured.
    pub fn resolve(static_dir: Option<&Path>) -> Option<Self> {
        if let Some(dir) = static_dir {
            return Some(Assets::Dir(dir.to_owned()));
        }
        #[cfg(feature = "embed-web")]
        {
            Some(Assets::Embedded)
        }
        #[cfg(not(feature = "embed-web"))]
        {
            None
        }
    }

    /// A short label for diagnostics (log lines only — never user-facing).
    fn label(&self) -> &'static str {
        match self {
            Assets::Dir(_) => "directory",
            #[cfg(feature = "embed-web")]
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
            #[cfg(feature = "embed-web")]
            Assets::Embedded => {
                let key = relative.to_str()?;
                WebDist::get(key).map(|file| file.data.into_owned())
            }
        }
    }
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
/// No `Cache-Control` is set, for parity with `ServeDir`'s own default
/// (content-hashed `_astro/*` assets could safely be marked immutable, but
/// that's a mode-independent improvement, tracked separately in
/// docs/ROADMAP.md rather than bundled into this mode's serving path).
#[cfg(feature = "embed-web")]
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

#[cfg(feature = "embed-web")]
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

    #[cfg(not(feature = "embed-web"))]
    #[test]
    fn resolve_should_be_none_when_unconfigured_and_not_embedded() {
        assert!(Assets::resolve(None).is_none());
    }

    #[cfg(feature = "embed-web")]
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

    #[cfg(feature = "embed-web")]
    #[test]
    fn hex_encode_should_match_a_known_digest() {
        assert_eq!(hex::encode([0u8; 32]), "0".repeat(64));
        assert_eq!(hex::encode([0xab; 32]), "ab".repeat(32));
    }
}
