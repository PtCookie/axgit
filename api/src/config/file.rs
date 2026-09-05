//! The optional TOML config file (docs/DECISIONS.md #87) — cgit's `cgitrc`
//! equivalent, for operators who would rather keep the whole deployment in
//! one commentable file than in a list of `AXGIT_*` environment variables.
//!
//! This module only *reads* the file into [`FileConfig`]; deciding which of
//! its values actually win is [`super::merge`]'s job, since that depends on
//! whether the matching flag or environment variable was set. Every field is
//! `Option`, so "absent from the file" and "present" stay distinguishable
//! all the way to the merge — a file that sets nothing is indistinguishable
//! from no file at all.
//!
//! Sections are organizational only: keys keep cgit's own `cgitrc` spelling
//! wherever cgit has one (`root-title`, `root-desc`, `root-readme`, `logo`,
//! `logo-link`, `favicon`, `repository-sort`), so a value copied out of an
//! existing `cgitrc` is recognizable, while axgit-only keys use the
//! kebab-case tail of their `AXGIT_*` variable.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Deserialize;

/// The config file's contents, one field per [`super::Config`] setting.
///
/// Deliberately *not* `deny_unknown_fields`: an unrecognized key is a
/// warning, not a startup failure (see [`unknown_keys`]).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct FileConfig {
    pub repo_root: Option<PathBuf>,
    pub static_dir: Option<PathBuf>,
    pub listen: Option<SocketAddr>,
    pub clone_url_base: Option<String>,
    /// Left as the raw string so it is validated by the same
    /// `super::parse_repository_sort` the flag uses, error text included.
    pub repository_sort: Option<String>,
    #[serde(default)]
    pub site: SiteSection,
    #[serde(default)]
    pub cache: CacheSection,
}

/// `[site]` — the site-wide branding/chrome settings (`site.rs`,
/// `branding.rs`).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SiteSection {
    pub root_title: Option<String>,
    pub root_desc: Option<String>,
    pub root_readme: Option<PathBuf>,
    pub logo: Option<String>,
    pub logo_link: Option<String>,
    pub favicon: Option<String>,
}

/// `[cache]` — the cache knobs (`cache.rs`). axgit-specific; cgit's own
/// `cache-*` options don't map onto these, so the names are axgit's.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct CacheSection {
    pub scan_ttl: Option<u64>,
    pub response_ttl: Option<u64>,
    pub response_max_bytes: Option<u64>,
}

/// Every key [`FileConfig`] accepts, by section. Hand-maintained and
/// exhaustive on purpose: `#[serde(deny_unknown_fields)]` would turn a stray
/// key into a startup failure (so a `cgitrc` pasted in wholesale, with its
/// `scan-path`/`enable-*`/`snapshots` keys, could never boot), and
/// `serde_ignored` would be a dependency for one warning. Same
/// "exhaustive and auditable on its own" stance as
/// `branding.rs::content_type_for_extension`.
const TOP_LEVEL_KEYS: &[&str] = &[
    "repo-root",
    "static-dir",
    "listen",
    "clone-url-base",
    "repository-sort",
];
const SITE_KEYS: &[&str] = &[
    "root-title",
    "root-desc",
    "root-readme",
    "logo",
    "logo-link",
    "favicon",
];
const CACHE_KEYS: &[&str] = &["scan-ttl", "response-ttl", "response-max-bytes"];

/// Reads and parses `path`. Both failure modes — unreadable file, malformed
/// TOML or a value of the wrong type — are hard errors that abort startup:
/// unlike the branding/readme paths (`site.rs`, `branding.rs`), which degrade
/// to `None` because one bad value only spoils one response, a config file
/// that can't be read means the whole deployment is running on settings the
/// operator didn't ask for.
pub fn load(path: &Path) -> anyhow::Result<FileConfig> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read config file {}", path.display()))?;
    parse(&text).with_context(|| format!("failed to parse config file {}", path.display()))
}

/// Parses config-file text, logging a warning for every key axgit doesn't
/// know. Parsed twice — once as a plain table to find those keys, once into
/// the typed struct — because serde's own unknown-field handling is
/// all-or-nothing (silently ignore, or fail); the file is a few dozen lines
/// read once at startup, so the second pass costs nothing worth optimizing.
pub fn parse(text: &str) -> anyhow::Result<FileConfig> {
    let table: toml::Table = toml::from_str(text)?;
    for key in unknown_keys(&table) {
        tracing::warn!(key = %key, "unknown key in the config file, ignoring it");
    }
    Ok(toml::from_str(text)?)
}

/// Keys present in `table` that no [`FileConfig`] field accepts, in
/// `section.key` form for the nested ones. Split out from the logging so it
/// can be asserted on directly in tests.
fn unknown_keys(table: &toml::Table) -> Vec<String> {
    let mut unknown = Vec::new();
    for (key, value) in table {
        let section_keys = match key.as_str() {
            "site" => SITE_KEYS,
            "cache" => CACHE_KEYS,
            _ => {
                if !TOP_LEVEL_KEYS.contains(&key.as_str()) {
                    unknown.push(key.clone());
                }
                continue;
            }
        };
        // A non-table `site`/`cache` has nothing to walk; the typed parse
        // below reports the shape error itself.
        let Some(section) = value.as_table() else {
            continue;
        };
        for inner in section.keys() {
            if !section_keys.contains(&inner.as_str()) {
                unknown.push(format!("{key}.{inner}"));
            }
        }
    }
    unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"
repo-root = "/srv/git"
static-dir = "/app/dist"
listen = "127.0.0.1:9090"
clone-url-base = "https://git.example.com"
repository-sort = "-idle"

[site]
root-title = "PtCookie Git"
root-desc = "self-hosted git"
root-readme = "/srv/git/README.md"
logo = "/srv/git/logo.svg"
logo-link = "https://example.com"
favicon = "/srv/git/favicon.png"

[cache]
scan-ttl = 30
response-ttl = 120
response-max-bytes = 1048576
"#;

    #[test]
    fn parse_should_read_every_key() {
        let config = parse(FULL).expect("expected a parse");

        assert_eq!(config.repo_root, Some(PathBuf::from("/srv/git")));
        assert_eq!(config.static_dir, Some(PathBuf::from("/app/dist")));
        assert_eq!(config.listen, Some("127.0.0.1:9090".parse().unwrap()));
        assert_eq!(
            config.clone_url_base.as_deref(),
            Some("https://git.example.com")
        );
        assert_eq!(config.repository_sort.as_deref(), Some("-idle"));

        assert_eq!(config.site.root_title.as_deref(), Some("PtCookie Git"));
        assert_eq!(config.site.root_desc.as_deref(), Some("self-hosted git"));
        assert_eq!(
            config.site.root_readme,
            Some(PathBuf::from("/srv/git/README.md"))
        );
        assert_eq!(config.site.logo.as_deref(), Some("/srv/git/logo.svg"));
        assert_eq!(
            config.site.logo_link.as_deref(),
            Some("https://example.com")
        );
        assert_eq!(config.site.favicon.as_deref(), Some("/srv/git/favicon.png"));

        assert_eq!(config.cache.scan_ttl, Some(30));
        assert_eq!(config.cache.response_ttl, Some(120));
        assert_eq!(config.cache.response_max_bytes, Some(1048576));
    }

    #[test]
    fn parse_should_leave_absent_keys_unset() {
        let config = parse("root-title = \"ignored\"\n[site]\nroot-title = \"Only This\"\n")
            .expect("expected a parse");

        assert_eq!(config.site.root_title.as_deref(), Some("Only This"));
        assert_eq!(config.repo_root, None);
        assert_eq!(config.site.root_desc, None);
        assert_eq!(config.cache.scan_ttl, None);
    }

    #[test]
    fn parse_should_accept_an_empty_file() {
        let config = parse("").expect("expected a parse");
        assert_eq!(config.repo_root, None);
        assert_eq!(config.site.root_title, None);
    }

    #[test]
    fn parse_should_ignore_unknown_keys() {
        // The cgitrc keys axgit has no equivalent for — a file carried over
        // from cgit still has to boot.
        let config = parse(
            "scan-path = \"/srv/git\"\nroot-title = \"Stray\"\n[site]\nroot-title = \"Kept\"\n",
        )
        .expect("expected a parse");
        assert_eq!(config.site.root_title.as_deref(), Some("Kept"));
    }

    #[test]
    fn unknown_keys_should_list_stray_top_level_keys_sections_and_nested_keys() {
        let table: toml::Table = toml::from_str(
            "scan-path = \"/srv/git\"\n[nope]\nwhatever = 1\n[site]\nenable-blame = true\n",
        )
        .expect("expected a table");

        let mut unknown = unknown_keys(&table);
        unknown.sort();
        assert_eq!(unknown, vec!["nope", "scan-path", "site.enable-blame"]);
    }

    #[test]
    fn unknown_keys_should_be_empty_for_a_fully_known_file() {
        let table: toml::Table = toml::from_str(FULL).expect("expected a table");
        assert!(unknown_keys(&table).is_empty());
    }

    #[test]
    fn parse_should_reject_a_malformed_listen_address() {
        assert!(parse("listen = \"not-an-address\"\n").is_err());
    }

    #[test]
    fn parse_should_reject_a_value_of_the_wrong_type() {
        assert!(parse("[cache]\nscan-ttl = \"sixty\"\n").is_err());
    }

    #[test]
    fn load_should_name_the_path_it_could_not_read() {
        let error = load(Path::new("/no/such/axgit.toml")).expect_err("expected an error");
        assert!(
            format!("{error}").contains("/no/such/axgit.toml"),
            "error should name the path: {error}"
        );
    }

    #[test]
    fn load_should_read_a_file_from_disk() {
        let dir = tempfile::tempdir().expect("failed to create tempdir");
        let path = dir.path().join("axgit.toml");
        std::fs::write(&path, "[site]\nroot-title = \"From File\"\n").expect("failed to write");

        let config = load(&path).expect("expected a load");
        assert_eq!(config.site.root_title.as_deref(), Some("From File"));
    }
}
