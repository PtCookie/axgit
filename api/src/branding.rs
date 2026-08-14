//! Site-wide logo/favicon config — cgit's `logo`/`logo-link`/`favicon`
//! (docs/DECISIONS.md #81). Like `site.rs`'s readme, this is operator
//! deployment config, not per-repository or user input: `AXGIT_LOGO` and
//! `AXGIT_FAVICON` each name either an `http(s)://` URL (used verbatim,
//! cgit's own behaviour) or a filesystem path axgit reads and serves itself
//! at request time — the file form is what actually works under the
//! `embed-web` single-binary deploy path, where there is no static
//! directory to drop an image into for a URL to point at.

use std::path::{Path, PathBuf};

use crate::repo::meta::is_http_url;

/// Cap on a logo/favicon file read — same "a misconfigured path pointing at
/// a huge file can't blow up response size" reasoning as
/// `site.rs::ROOT_README_LIMIT`, sized for an icon/logo image rather than a
/// text readme.
pub const BRANDING_ASSET_LIMIT: u64 = 1024 * 1024;

/// Where a configured logo/favicon value points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrandingAsset {
    /// Used verbatim in the emitted `href`/`src` — cgit's own behaviour.
    Url(String),
    /// Read from disk and served by axgit itself, at the fixed endpoint the
    /// caller passes to [`BrandingAsset::href`].
    File(PathBuf),
}

impl BrandingAsset {
    /// `http://`/`https://` is a [`Self::Url`]; anything else is a
    /// filesystem path. Unambiguous — a `cgitrc`-style bare relative path
    /// never starts with a scheme.
    pub fn parse(raw: &str) -> Self {
        if is_http_url(raw) {
            Self::Url(raw.to_owned())
        } else {
            Self::File(PathBuf::from(raw))
        }
    }

    /// The `href`/`src` this asset renders as: the URL itself, or `served_at`
    /// — axgit's own route for the file form, which never varies with the
    /// configured path.
    pub fn href(&self, served_at: &str) -> String {
        match self {
            Self::Url(url) => url.clone(),
            Self::File(_) => served_at.to_owned(),
        }
    }

    /// The favicon `<link rel="icon" type="…">` MIME, when knowable ahead of
    /// actually serving the file — used to build the `<link>` server-side
    /// (`shell.rs::favicon_head_link`), since [`Self::href`] always resolves
    /// the file form to the fixed, extensionless `/api/v1/site/favicon`
    /// route with no extension of its own to guess from. The URL form
    /// guesses from the URL's own extension instead (query/fragment
    /// stripped first, so `?v=2` doesn't defeat the match); either form
    /// falls back to `None` — no `type=` at all, same as an ordinary
    /// extensionless favicon link — for an unrecognized or absent
    /// extension, and the browser sniffs.
    pub fn content_type(&self) -> Option<&'static str> {
        match self {
            Self::Url(url) => {
                let path = url.split(['?', '#']).next().unwrap_or(url);
                content_type_for_extension(Path::new(path))
            }
            Self::File(path) => content_type_for_extension(path),
        }
    }
}

/// Validates `AXGIT_LOGO_LINK`: only an `http(s)://` URL or a root-relative
/// path (`/…`, not `//…` — a protocol-relative URL is an open redirect to
/// any origin) is safe to land directly in an `<a href>`, the same rule
/// `repo/meta.rs::is_http_url` enforces for `homepage` plus the root-relative
/// allowance `repo/submodule.rs`'s `module-link` templates already make.
/// Anything else degrades to unset rather than a scan-time error over one
/// misconfigured value.
pub fn sanitize_logo_link(raw: &str) -> Option<String> {
    if is_http_url(raw) || (raw.starts_with('/') && !raw.starts_with("//")) {
        Some(raw.to_owned())
    } else {
        tracing::warn!(
            value = raw,
            "AXGIT_LOGO_LINK is neither an http(s) URL nor a root-relative path, ignoring it",
        );
        None
    }
}

/// A successfully loaded branding asset.
pub struct LoadedAsset {
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

/// Extension allowlist, image types only. This is a security boundary, not
/// a convenience: these bytes are served same-origin with no further
/// inspection, so an operator accidentally pointing `AXGIT_LOGO`/
/// `AXGIT_FAVICON` at, say, an `.html` file must never become stored XSS.
/// Deliberately not `mime_guess` (already a dependency, used for repository
/// file content types in `handlers/files.rs`): that crate's table is far
/// broader than "safe to serve as a site image", and this list needs to stay
/// exhaustive and auditable on its own.
fn content_type_for_extension(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        _ => return None,
    })
}

/// Reads a configured branding file. `None` covers every way this can fail
/// to produce something servable — an unrecognized extension, a missing or
/// unreadable file, or one over [`BRANDING_ASSET_LIMIT`] — each logged, but
/// this is deployment config the requesting client didn't cause, so there's
/// nothing to answer with other than the handler's own 404 (mirrors
/// `site.rs::read_root_readme`'s degrade-to-`None` stance).
pub async fn read_asset(path: &Path) -> Option<LoadedAsset> {
    let Some(content_type) = content_type_for_extension(path) else {
        tracing::warn!(
            path = %path.display(),
            "branding asset has an unrecognized extension, serving 404",
        );
        return None;
    };

    let metadata = tokio::fs::metadata(path).await.ok()?;
    if metadata.len() > BRANDING_ASSET_LIMIT {
        tracing::warn!(
            path = %path.display(),
            size = metadata.len(),
            limit = BRANDING_ASSET_LIMIT,
            "branding asset exceeds the size limit, serving 404",
        );
        return None;
    }

    let body = tokio::fs::read(path).await.ok()?;
    Some(LoadedAsset { content_type, body })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_should_treat_http_schemes_as_urls() {
        assert_eq!(
            BrandingAsset::parse("https://example.net/logo.svg"),
            BrandingAsset::Url("https://example.net/logo.svg".to_owned())
        );
        assert_eq!(
            BrandingAsset::parse("http://example.net/logo.svg"),
            BrandingAsset::Url("http://example.net/logo.svg".to_owned())
        );
    }

    #[test]
    fn parse_should_treat_anything_else_as_a_file_path() {
        assert_eq!(
            BrandingAsset::parse("/srv/git/logo.svg"),
            BrandingAsset::File(PathBuf::from("/srv/git/logo.svg"))
        );
        assert_eq!(
            BrandingAsset::parse("relative/logo.svg"),
            BrandingAsset::File(PathBuf::from("relative/logo.svg"))
        );
    }

    #[test]
    fn href_should_use_the_url_verbatim_or_the_served_at_path() {
        let url = BrandingAsset::Url("https://example.net/logo.svg".to_owned());
        assert_eq!(
            url.href("/api/v1/site/logo"),
            "https://example.net/logo.svg"
        );

        let file = BrandingAsset::File(PathBuf::from("/srv/git/logo.svg"));
        assert_eq!(file.href("/api/v1/site/logo"), "/api/v1/site/logo");
    }

    #[test]
    fn content_type_should_resolve_from_the_file_extension() {
        let file = BrandingAsset::File(PathBuf::from("/srv/git/favicon.png"));
        assert_eq!(file.content_type(), Some("image/png"));

        let unrecognized = BrandingAsset::File(PathBuf::from("/srv/git/favicon.html"));
        assert_eq!(unrecognized.content_type(), None);
    }

    #[test]
    fn content_type_should_resolve_from_the_url_extension_ignoring_query_and_fragment() {
        let url = BrandingAsset::Url("https://example.net/favicon.svg?v=2".to_owned());
        assert_eq!(url.content_type(), Some("image/svg+xml"));

        let no_extension = BrandingAsset::Url("https://example.net/favicon".to_owned());
        assert_eq!(no_extension.content_type(), None);
    }

    #[test]
    fn sanitize_logo_link_should_accept_http_urls_and_root_relative_paths() {
        assert_eq!(
            sanitize_logo_link("https://example.net"),
            Some("https://example.net".to_owned())
        );
        assert_eq!(sanitize_logo_link("/about"), Some("/about".to_owned()));
    }

    #[test]
    fn sanitize_logo_link_should_reject_everything_else() {
        assert_eq!(sanitize_logo_link("javascript:alert(1)"), None);
        assert_eq!(sanitize_logo_link("//evil.example"), None);
        assert_eq!(sanitize_logo_link("relative/path"), None);
    }

    #[test]
    fn content_type_for_extension_should_match_the_image_allowlist() {
        assert_eq!(
            content_type_for_extension(Path::new("logo.svg")),
            Some("image/svg+xml")
        );
        assert_eq!(
            content_type_for_extension(Path::new("logo.PNG")),
            Some("image/png")
        );
        assert_eq!(
            content_type_for_extension(Path::new("favicon.ico")),
            Some("image/x-icon")
        );
        assert_eq!(
            content_type_for_extension(Path::new("logo.jpeg")),
            Some("image/jpeg")
        );
        assert_eq!(content_type_for_extension(Path::new("logo.html")), None);
        assert_eq!(content_type_for_extension(Path::new("logo")), None);
    }

    #[tokio::test]
    async fn read_asset_should_return_none_for_a_missing_file() {
        assert!(read_asset(Path::new("/no/such/logo.svg")).await.is_none());
    }

    #[tokio::test]
    async fn read_asset_should_return_none_for_an_unrecognized_extension() {
        let dir = tempfile::tempdir().expect("failed to create tempdir");
        let path = dir.path().join("logo.html");
        std::fs::write(&path, b"<html></html>").expect("failed to write file");
        assert!(read_asset(&path).await.is_none());
    }

    #[tokio::test]
    async fn read_asset_should_read_a_small_svg() {
        let dir = tempfile::tempdir().expect("failed to create tempdir");
        let path = dir.path().join("logo.svg");
        std::fs::write(&path, b"<svg></svg>").expect("failed to write file");

        let asset = read_asset(&path).await.expect("expected Some");
        assert_eq!(asset.content_type, "image/svg+xml");
        assert_eq!(asset.body, b"<svg></svg>");
    }

    #[tokio::test]
    async fn read_asset_should_return_none_when_over_the_limit() {
        let dir = tempfile::tempdir().expect("failed to create tempdir");
        let path = dir.path().join("logo.png");
        std::fs::write(&path, vec![0u8; BRANDING_ASSET_LIMIT as usize + 1])
            .expect("failed to write file");

        assert!(read_asset(&path).await.is_none());
    }
}
