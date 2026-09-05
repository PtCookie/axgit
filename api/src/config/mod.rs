use std::net::SocketAddr;
use std::path::PathBuf;

use clap::parser::ValueSource;
use clap::{ArgMatches, CommandFactory, FromArgMatches, Parser};

use crate::repo::sort::RepoOrder;

mod file;

use file::FileConfig;

/// Config file consulted when neither `--config` nor `AXGIT_CONFIG` names
/// one. Its absence is not an error — axgit has never needed a config file
/// and still doesn't — unlike a path the operator named explicitly, which
/// has to exist.
pub const DEFAULT_CONFIG_PATH: &str = "/etc/axgit/axgit.toml";

/// Axgit — read-only web frontend for bare Git repositories.
#[derive(Parser, Debug, Clone)]
#[command(name = "axgit", version)]
pub struct Config {
    /// TOML config file holding any of the settings below (docs/DECISIONS.md
    /// #87). Unset falls back to [`DEFAULT_CONFIG_PATH`] when that file
    /// exists. A flag or environment variable always beats the file; the
    /// file only fills in what neither of them set.
    #[arg(long, env = "AXGIT_CONFIG")]
    pub config: Option<PathBuf>,

    /// Directory containing bare repositories (`*.git`).
    #[arg(long, env = "AXGIT_REPO_ROOT", default_value = "/srv/git")]
    pub repo_root: PathBuf,

    /// Static frontend build (web/dist) to serve at `/`.
    #[arg(long, env = "AXGIT_STATIC_DIR")]
    pub static_dir: Option<PathBuf>,

    /// Socket address to listen on.
    #[arg(long, env = "AXGIT_LISTEN", default_value = "0.0.0.0:8080")]
    pub listen: SocketAddr,

    /// Base URL used when displaying clone URLs.
    #[arg(long, env = "AXGIT_CLONE_URL_BASE")]
    pub clone_url_base: Option<String>,

    /// Repository scan cache TTL in seconds.
    #[arg(long, env = "AXGIT_CACHE_SCAN_TTL", default_value_t = 60)]
    pub cache_scan_ttl_secs: u64,

    /// Response cache TTL in seconds. The cache is invalidated by HEAD/agefile
    /// validators; the TTL only bounds staleness for out-of-band changes the
    /// validators cannot see (e.g. a manual config edit).
    #[arg(long, env = "AXGIT_CACHE_RESPONSE_TTL", default_value_t = 300)]
    pub cache_response_ttl_secs: u64,

    /// Response cache capacity in bytes.
    #[arg(long, env = "AXGIT_CACHE_RESPONSE_MAX_BYTES", default_value_t = 32 * 1024 * 1024)]
    pub cache_response_max_bytes: u64,

    /// Default repository index sort order (`name`, `desc`, `owner`, `idle`,
    /// `section`, optionally `-`-prefixed) — cgit's `repository-sort`. A
    /// request's own `?sort=` overrides this.
    #[arg(
        long,
        env = "AXGIT_REPOSITORY_SORT",
        default_value = "name",
        value_parser = parse_repository_sort,
    )]
    pub repository_sort: RepoOrder,

    /// Site-wide title, shown as the header brand and browser tab on non-
    /// repository pages (falls back to "Axgit" when unset) — cgit's
    /// `root-title`.
    #[arg(long, env = "AXGIT_ROOT_TITLE")]
    pub root_title: Option<String>,

    /// Site-wide description, shown on the index page and in `<meta
    /// name="description">` — cgit's `root-desc`.
    #[arg(long, env = "AXGIT_ROOT_DESC")]
    pub root_desc: Option<String>,

    /// Path to a markdown/reStructuredText/plain-text file rendered on the
    /// index page — cgit's `root-readme`. Read from the filesystem at
    /// request time; operator config, not user input, so it is not subject
    /// to any traversal check.
    #[arg(long, env = "AXGIT_ROOT_README")]
    pub root_readme: Option<PathBuf>,

    /// Site logo, shown beside the header brand — cgit's `logo`. Either an
    /// `http(s)://` URL (used verbatim) or a filesystem path axgit reads and
    /// serves itself at `GET /api/v1/site/logo` (`branding.rs`,
    /// docs/DECISIONS.md #81) — the path form is what works under the
    /// default single-binary deploy, which has no static directory to
    /// drop an image into.
    #[arg(long, env = "AXGIT_LOGO")]
    pub logo: Option<String>,

    /// Where the logo links to, when set — cgit's `logo-link`. Must be an
    /// `http(s)://` URL or a root-relative path; anything else is ignored
    /// (`branding.rs::sanitize_logo_link`). Unset falls back to `/`.
    #[arg(long, env = "AXGIT_LOGO_LINK")]
    pub logo_link: Option<String>,

    /// Site favicon, replacing axgit's own default — cgit's `favicon`. Same
    /// URL-or-path rule as `logo`, served at `GET /api/v1/site/favicon` when
    /// given a path.
    #[arg(long, env = "AXGIT_FAVICON")]
    pub favicon: Option<String>,
}

impl Config {
    /// Parses flags and environment variables, then fills whatever is still
    /// sitting at its built-in default from the TOML config file, if there is
    /// one. Precedence is flag > environment variable > config file > default
    /// (docs/DECISIONS.md #87) — the container image bakes `AXGIT_*` into its
    /// `ENV`, so a config file must never be able to silently reconfigure an
    /// existing deployment out from under it.
    ///
    /// Expects a tracing subscriber to already be installed: an unknown key
    /// in the file is a warning, not an error, and would otherwise vanish.
    ///
    /// `self.config` comes back holding the path that was actually applied
    /// (`None` when no file was read), so the caller can log it.
    pub fn load() -> anyhow::Result<Self> {
        let matches = Self::command().get_matches();
        let mut config = Self::from_arg_matches(&matches)?;

        let Some(path) = config.resolve_config_path() else {
            return Ok(config);
        };
        let file = file::load(&path)?;
        merge(&mut config, file, |id| is_explicit(&matches, id))?;
        config.config = Some(path);
        Ok(config)
    }

    /// The config file to read: the one named explicitly (whether or not it
    /// exists — a named-but-missing file is an error `file::load` reports,
    /// not something to silently skip), else [`DEFAULT_CONFIG_PATH`] if it
    /// happens to be there.
    fn resolve_config_path(&self) -> Option<PathBuf> {
        if let Some(path) = &self.config {
            return Some(path.clone());
        }
        let default = PathBuf::from(DEFAULT_CONFIG_PATH);
        default.is_file().then_some(default)
    }
}

/// Whether `id`'s value came from the command line or the environment,
/// rather than from clap's own `default_value` (or from nothing at all, for
/// the `Option` fields). Only the fields this returns `false` for are open
/// to the config file — that predicate *is* the precedence rule.
fn is_explicit(matches: &ArgMatches, id: &str) -> bool {
    matches!(
        matches.value_source(id),
        Some(ValueSource::CommandLine | ValueSource::EnvVariable)
    )
}

/// Applies `file` to every field `is_explicit` reports as unset. `is_explicit`
/// is a predicate over clap arg ids (the derive uses the field idents, e.g.
/// `cache_scan_ttl_secs`, not the flag names) rather than an `&ArgMatches`, so
/// the precedence rule can be tested without a process environment to set up.
fn merge(
    config: &mut Config,
    file: FileConfig,
    is_explicit: impl Fn(&str) -> bool,
) -> anyhow::Result<()> {
    if !is_explicit("repo_root")
        && let Some(value) = file.repo_root
    {
        config.repo_root = value;
    }
    if !is_explicit("static_dir")
        && let Some(value) = file.static_dir
    {
        config.static_dir = Some(value);
    }
    if !is_explicit("listen")
        && let Some(value) = file.listen
    {
        config.listen = value;
    }
    if !is_explicit("clone_url_base")
        && let Some(value) = file.clone_url_base
    {
        config.clone_url_base = Some(value);
    }
    if !is_explicit("repository_sort")
        && let Some(value) = file.repository_sort
    {
        // Same validation, and the same error text, as the flag's own
        // `value_parser` — a bad `repository-sort` aborts startup either way.
        config.repository_sort = parse_repository_sort(&value).map_err(anyhow::Error::msg)?;
    }

    if !is_explicit("root_title")
        && let Some(value) = file.site.root_title
    {
        config.root_title = Some(value);
    }
    if !is_explicit("root_desc")
        && let Some(value) = file.site.root_desc
    {
        config.root_desc = Some(value);
    }
    if !is_explicit("root_readme")
        && let Some(value) = file.site.root_readme
    {
        config.root_readme = Some(value);
    }
    if !is_explicit("logo")
        && let Some(value) = file.site.logo
    {
        config.logo = Some(value);
    }
    if !is_explicit("logo_link")
        && let Some(value) = file.site.logo_link
    {
        config.logo_link = Some(value);
    }
    if !is_explicit("favicon")
        && let Some(value) = file.site.favicon
    {
        config.favicon = Some(value);
    }

    if !is_explicit("cache_scan_ttl_secs")
        && let Some(value) = file.cache.scan_ttl
    {
        config.cache_scan_ttl_secs = value;
    }
    if !is_explicit("cache_response_ttl_secs")
        && let Some(value) = file.cache.response_ttl
    {
        config.cache_response_ttl_secs = value;
    }
    if !is_explicit("cache_response_max_bytes")
        && let Some(value) = file.cache.response_max_bytes
    {
        config.cache_response_max_bytes = value;
    }

    Ok(())
}

fn parse_repository_sort(raw: &str) -> Result<RepoOrder, String> {
    RepoOrder::parse(raw).map_err(|_| {
        format!("sort must be one of name, desc, owner, idle, section, optionally prefixed with '-' (got '{raw}')")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every field at the value `Config::load` would have produced from an
    /// empty command line and an empty environment.
    fn default_config() -> Config {
        Config {
            config: None,
            repo_root: PathBuf::from("/srv/git"),
            static_dir: None,
            listen: "0.0.0.0:8080".parse().unwrap(),
            clone_url_base: None,
            cache_scan_ttl_secs: 60,
            cache_response_ttl_secs: 300,
            cache_response_max_bytes: 32 * 1024 * 1024,
            repository_sort: RepoOrder::default(),
            root_title: None,
            root_desc: None,
            root_readme: None,
            logo: None,
            logo_link: None,
            favicon: None,
        }
    }

    const FULL_FILE: &str = r#"
repo-root = "/var/git"
static-dir = "/app/dist"
listen = "127.0.0.1:9090"
clone-url-base = "https://git.example.com"
repository-sort = "-idle"

[site]
root-title = "PtCookie Git"
root-desc = "self-hosted git"
root-readme = "/var/git/README.md"
logo = "/var/git/logo.svg"
logo-link = "https://example.com"
favicon = "/var/git/favicon.png"

[cache]
scan-ttl = 30
response-ttl = 120
response-max-bytes = 1048576
"#;

    #[test]
    fn merge_should_fill_every_field_left_at_its_default() {
        let mut config = default_config();
        let file = file::parse(FULL_FILE).expect("expected a parse");

        merge(&mut config, file, |_| false).expect("expected a merge");

        assert_eq!(config.repo_root, PathBuf::from("/var/git"));
        assert_eq!(config.static_dir, Some(PathBuf::from("/app/dist")));
        assert_eq!(config.listen, "127.0.0.1:9090".parse().unwrap());
        assert_eq!(
            config.clone_url_base.as_deref(),
            Some("https://git.example.com")
        );
        assert_eq!(
            config.repository_sort,
            RepoOrder::parse("-idle").expect("expected a sort")
        );
        assert_eq!(config.root_title.as_deref(), Some("PtCookie Git"));
        assert_eq!(config.root_desc.as_deref(), Some("self-hosted git"));
        assert_eq!(
            config.root_readme,
            Some(PathBuf::from("/var/git/README.md"))
        );
        assert_eq!(config.logo.as_deref(), Some("/var/git/logo.svg"));
        assert_eq!(config.logo_link.as_deref(), Some("https://example.com"));
        assert_eq!(config.favicon.as_deref(), Some("/var/git/favicon.png"));
        assert_eq!(config.cache_scan_ttl_secs, 30);
        assert_eq!(config.cache_response_ttl_secs, 120);
        assert_eq!(config.cache_response_max_bytes, 1048576);
    }

    #[test]
    fn merge_should_leave_flag_and_env_provided_fields_alone() {
        let mut config = default_config();
        config.repo_root = PathBuf::from("/from/flag");
        config.root_title = Some("From Env".to_owned());
        let file = file::parse(FULL_FILE).expect("expected a parse");

        // Stands in for `--repo-root /from/flag AXGIT_ROOT_TITLE="From Env"`.
        let explicit = ["repo_root", "root_title"];
        merge(&mut config, file, |id| explicit.contains(&id)).expect("expected a merge");

        assert_eq!(config.repo_root, PathBuf::from("/from/flag"));
        assert_eq!(config.root_title.as_deref(), Some("From Env"));
        // Everything else still takes the file's value.
        assert_eq!(config.root_desc.as_deref(), Some("self-hosted git"));
        assert_eq!(config.listen, "127.0.0.1:9090".parse().unwrap());
    }

    #[test]
    fn merge_should_be_a_no_op_for_an_empty_file() {
        let mut config = default_config();
        let file = file::parse("").expect("expected a parse");

        merge(&mut config, file, |_| false).expect("expected a merge");

        assert_eq!(config.repo_root, PathBuf::from("/srv/git"));
        assert_eq!(config.listen, "0.0.0.0:8080".parse().unwrap());
        assert_eq!(config.root_title, None);
        assert_eq!(config.cache_scan_ttl_secs, 60);
    }

    #[test]
    fn merge_should_reject_an_invalid_repository_sort() {
        let mut config = default_config();
        let file = file::parse("repository-sort = \"nonsense\"\n").expect("expected a parse");

        let error = merge(&mut config, file, |_| false).expect_err("expected an error");
        assert!(
            format!("{error}").contains("nonsense"),
            "error should quote the offending value: {error}"
        );
    }

    #[test]
    fn resolve_config_path_should_prefer_an_explicit_path() {
        let mut config = default_config();
        config.config = Some(PathBuf::from("/no/such/axgit.toml"));

        assert_eq!(
            config.resolve_config_path(),
            Some(PathBuf::from("/no/such/axgit.toml"))
        );
    }

    #[test]
    fn resolve_config_path_should_fall_back_to_the_default_only_when_it_exists() {
        // Branches on the host rather than asserting one way: a machine
        // actually running axgit has /etc/axgit/axgit.toml, and the test
        // suite has to pass there too.
        let default = PathBuf::from(DEFAULT_CONFIG_PATH);
        let expected = default.is_file().then_some(default);
        assert_eq!(default_config().resolve_config_path(), expected);
    }
}
