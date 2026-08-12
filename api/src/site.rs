//! Site-wide metadata (`GET /api/v1/site`) — cgit's `root-title`/`root-desc`/
//! `root-readme` (docs/DECISIONS.md #70). Unlike everything else in this
//! api, none of this is per-repository: it's read once per request from
//! server config plus, for the readme, whatever the filesystem currently
//! holds at the path that config names.

use std::path::Path;

use serde::Serialize;
use utoipa::ToSchema;

use crate::repo::readme::ReadmeFormat;

/// Cap on the `AXGIT_ROOT_README` read — same limit the per-repository
/// readme applies to a blob (`repo/blob.rs::BLOB_CONTENT_LIMIT`), so a
/// misconfigured path pointing at a huge file can't blow up response size.
pub const ROOT_README_LIMIT: u64 = 512 * 1024;

const DEFAULT_TITLE: &str = "Axgit";

#[derive(Debug, Serialize, ToSchema)]
pub struct SiteReadme {
    pub format: ReadmeFormat,
    pub content: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SiteInfo {
    /// `AXGIT_ROOT_TITLE`, falling back to `"Axgit"` when unset — always
    /// present, unlike `description`/`readme`.
    #[schema(example = "Axgit")]
    pub title: String,
    /// `AXGIT_ROOT_DESC`. `null` when unset.
    #[schema(required = true)]
    pub description: Option<String>,
    /// From `AXGIT_ROOT_README`. `null` when unset, unreadable, over the
    /// size limit, or not valid UTF-8.
    #[schema(required = true)]
    pub readme: Option<SiteReadme>,
}

/// `AXGIT_ROOT_TITLE` if set, else the default. A separate function (rather
/// than inlining `.unwrap_or_else`) so the default lives in one place callers
/// besides the handler (tests) can reuse.
pub fn effective_title(configured: Option<&str>) -> String {
    configured
        .map(str::to_owned)
        .unwrap_or_else(|| DEFAULT_TITLE.to_owned())
}

/// Guesses a render format from `AXGIT_ROOT_README`'s extension — the
/// filesystem analogue of `repo/readme.rs::CANDIDATES`' name-based guess,
/// since an operator-chosen path has no fixed candidate list to match
/// against. Anything other than `.md`/`.markdown`/`.rst` is treated as plain
/// text, the same fallback `repo/readme.rs` uses for `README.txt`/`README`.
fn guess_format(path: &Path) -> ReadmeFormat {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown") => {
            ReadmeFormat::Markdown
        }
        Some(ext) if ext.eq_ignore_ascii_case("rst") => ReadmeFormat::Rst,
        _ => ReadmeFormat::Plain,
    }
}

/// Reads `AXGIT_ROOT_README`. A missing/unreadable file, one over
/// [`ROOT_README_LIMIT`], or non-UTF-8 content all degrade to `None` —
/// logged, but this is deployment config, not something the requesting
/// client caused, so there's nothing to answer with a 4xx/5xx over; the rest
/// of `GET /api/v1/site` (`title`/`description`) is unaffected either way.
pub async fn read_root_readme(path: &Path) -> Option<SiteReadme> {
    let metadata = tokio::fs::metadata(path).await.ok()?;
    if metadata.len() > ROOT_README_LIMIT {
        tracing::warn!(
            path = %path.display(),
            size = metadata.len(),
            limit = ROOT_README_LIMIT,
            "AXGIT_ROOT_README exceeds the size limit, serving null",
        );
        return None;
    }
    let bytes = tokio::fs::read(path).await.ok()?;
    let content = String::from_utf8(bytes).ok()?;
    Some(SiteReadme {
        format: guess_format(path),
        content,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_title_should_fall_back_to_axgit() {
        assert_eq!(effective_title(None), "Axgit");
        assert_eq!(effective_title(Some("PtCookie Git")), "PtCookie Git");
    }

    #[test]
    fn guess_format_should_key_off_the_extension() {
        assert_eq!(
            guess_format(Path::new("/srv/README.md")),
            ReadmeFormat::Markdown
        );
        assert_eq!(
            guess_format(Path::new("/srv/README.MARKDOWN")),
            ReadmeFormat::Markdown
        );
        assert_eq!(
            guess_format(Path::new("/srv/README.rst")),
            ReadmeFormat::Rst
        );
        assert_eq!(
            guess_format(Path::new("/srv/README.txt")),
            ReadmeFormat::Plain
        );
        assert_eq!(guess_format(Path::new("/srv/README")), ReadmeFormat::Plain);
    }

    #[tokio::test]
    async fn read_root_readme_should_return_none_for_a_missing_file() {
        assert!(
            read_root_readme(Path::new("/no/such/file.md"))
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn read_root_readme_should_read_a_small_markdown_file() {
        let dir = tempfile::tempdir().expect("failed to create tempdir");
        let path = dir.path().join("README.md");
        std::fs::write(&path, "# Hello\n").expect("failed to write file");

        let readme = read_root_readme(&path).await.expect("expected Some");
        assert_eq!(readme.format, ReadmeFormat::Markdown);
        assert_eq!(readme.content, "# Hello\n");
    }

    #[tokio::test]
    async fn read_root_readme_should_return_none_when_over_the_limit() {
        let dir = tempfile::tempdir().expect("failed to create tempdir");
        let path = dir.path().join("README.md");
        std::fs::write(&path, vec![b'a'; ROOT_README_LIMIT as usize + 1])
            .expect("failed to write file");

        assert!(read_root_readme(&path).await.is_none());
    }

    #[tokio::test]
    async fn read_root_readme_should_return_none_for_non_utf8_content() {
        let dir = tempfile::tempdir().expect("failed to create tempdir");
        let path = dir.path().join("README.md");
        std::fs::write(&path, [0xff, 0xfe, 0xfd]).expect("failed to write file");

        assert!(read_root_readme(&path).await.is_none());
    }
}
