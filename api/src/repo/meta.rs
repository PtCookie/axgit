use std::fs;
use std::time::SystemTime;

use git2::Repository;
use jiff::fmt::strtime;
use jiff::tz::{Offset, TimeZone};
use jiff::{Timestamp, Zoned};

use super::RepoInfo;

const RFC3339_OUT: &str = "%Y-%m-%dT%H:%M:%S%:z";

/// Freshness validator for cached responses (api/README.md#caching):
/// a cache entry is served only while the repository still produces the same
/// validator, so a push (HEAD move or agefile touch) invalidates immediately.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Validator {
    /// HEAD commit id. `None` for empty repositories (unborn HEAD).
    pub head: Option<git2::Oid>,
    /// Raw mtime of the agefile (`info/web/last-modified`), which the
    /// post-receive hook touches on every push. `None` when absent.
    pub agefile_mtime: Option<SystemTime>,
}

/// Reads the current validator. Cheap (one ref lookup + one stat), so it runs
/// on every cache hit.
pub fn validator(repo: &Repository) -> Validator {
    Validator {
        head: repo.head().ok().and_then(|head| head.target()),
        agefile_mtime: fs::metadata(repo.path().join("info/web/last-modified"))
            .and_then(|metadata| metadata.modified())
            .ok(),
    }
}

/// Reads list metadata for one repository. `name` is the directory name
/// without the `.git` suffix.
pub fn read_repo_info(repo: &Repository, name: &str) -> RepoInfo {
    // git2 requires a snapshot to read string values from a live config.
    let config = repo.config().and_then(|mut cfg| cfg.snapshot()).ok();
    // An empty (or whitespace-only) value is treated the same as unset —
    // `section`/`owner`/`desc` land straight in the API response and the web
    // list's section grouping, where a blank string would otherwise render
    // as a group with no heading (docs/DECISIONS.md #86) instead of falling
    // into the same bucket as a truly-unset section.
    let meta = |key: &str| {
        config
            .as_ref()
            .and_then(|cfg| config_value(cfg, key))
            .filter(|value| !value.trim().is_empty())
    };

    RepoInfo {
        name: name.to_owned(),
        section: meta("section"),
        owner: meta("owner"),
        description: meta("desc"),
        homepage: meta("homepage").filter(|url| is_http_url(url)),
        default_branch: default_branch(repo),
        last_modified: last_modified(repo),
    }
}

/// `[axgit]` section wins over `[cgit]` (docs/DECISIONS.md #5).
fn config_value(cfg: &git2::Config, key: &str) -> Option<String> {
    ["axgit", "cgit"]
        .iter()
        .find_map(|section| cfg.get_string(&format!("{section}.{key}")).ok())
}

/// `homepage` is operator config, but it lands directly in an `<a href>` —
/// unlike `section`/`owner`/`desc`, which are always rendered as text, a
/// `javascript:` value here would be a stored XSS. Only `http://`/`https://`
/// pass; anything else reads as if `homepage` were unset (docs/DECISIONS.md
/// #67), rather than a scan-time error over a single misconfigured repo.
///
/// The shared "does this belong in an `href`" primitive: `homepage` uses it
/// as-is, and `repo::submodule`'s `.gitmodules` fallback reuses it verbatim
/// (docs/DECISIONS.md #72) — a `.gitmodules` `url` is commonly a local
/// filesystem path (`/srv/git/dep.git`) or an SSH remote, neither of which
/// belongs in an href, so unlike a `module-link` *template* result
/// (`submodule::is_link_href`, deliberately looser), a `.gitmodules` URL gets
/// no root-relative allowance.
pub(crate) fn is_http_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// The `module-link` template for a gitlink at `path`, most specific first:
/// `axgit.<path>.module-link` → `cgit.<path>.module-link` →
/// `axgit.module-link` → `cgit.module-link` (docs/DECISIONS.md #72). Both
/// levels go through [`config_value`], so the `[axgit]`-wins-over-`[cgit]`
/// rule (docs/DECISIONS.md #5) is inherited rather than restated — which
/// means path specificity is checked *before* section precedence: a
/// `[cgit "<path>"]` entry beats a repo-wide `[axgit]` one. The first level
/// that yields a value stops the search, even if that value is empty or
/// otherwise unusable — an empty per-path value is how an operator
/// suppresses a repo-wide template for one path.
pub(crate) fn module_link_template(cfg: &git2::Config, path: &str) -> Option<String> {
    config_value(cfg, &format!("{path}.module-link")).or_else(|| config_value(cfg, "module-link"))
}

/// Same `[axgit]`-wins-over-`[cgit]` precedence as [`config_value`], but for
/// a boolean flag (docs/DECISIONS.md #66) — cgit's own `repo.hide`/
/// `repo.ignore` are booleans in `cgitrc`, and `git2::Config::get_bool`
/// accepts the same spellings git itself does (`true`/`false`, `yes`/`no`,
/// `on`/`off`, `1`/`0`). Absent or unparseable defaults to `false`.
fn config_flag(cfg: &git2::Config, key: &str) -> bool {
    ["axgit", "cgit"]
        .iter()
        .find_map(|section| cfg.get_bool(&format!("{section}.{key}")).ok())
        .unwrap_or(false)
}

/// Whether the repository belongs in `GET /api/v1/repos` — `false` when
/// either `hide` or `ignore` is set (docs/DECISIONS.md #66). Direct
/// access (`GET /repos/{name}`, clone) has its own, narrower check
/// ([`is_ignored`]) — a *hidden* repository stays fully reachable by name,
/// only dropped from the listing.
pub fn should_list(repo: &Repository) -> bool {
    let config = repo.config().and_then(|mut cfg| cfg.snapshot()).ok();
    let flag = |key: &str| config.as_ref().is_some_and(|cfg| config_flag(cfg, key));
    !flag("hide") && !flag("ignore")
}

/// Whether the repository is `ignore`d — not reachable at all, not even by
/// direct path. Checked by [`super::open::open_named`] itself, so it blocks
/// every per-repo handler and Smart HTTP alike, the same choke point that
/// already rejects a malformed `{repo}` name.
pub fn is_ignored(repo: &Repository) -> bool {
    let config = repo.config().and_then(|mut cfg| cfg.snapshot()).ok();
    config
        .as_ref()
        .is_some_and(|cfg| config_flag(cfg, "ignore"))
}

/// The configured `defbranch` wins over HEAD's own shorthand
/// (docs/DECISIONS.md #68) — cgit's own `defbranch`.
fn default_branch(repo: &Repository) -> Option<String> {
    configured_default_branch(repo).or_else(|| repo.head().ok()?.shorthand().map(str::to_owned))
}

/// The repo's `defbranch` config value, if set and if it names an existing
/// local branch. A stale/misconfigured value (renamed or deleted branch)
/// degrades to `None` — callers fall back to HEAD — rather than making every
/// "no ref given" request fail because of one bad config value.
pub(crate) fn configured_default_branch(repo: &Repository) -> Option<String> {
    let config = repo.config().and_then(|mut cfg| cfg.snapshot()).ok();
    let name = config
        .as_ref()
        .and_then(|cfg| config_value(cfg, "defbranch"))?;
    repo.find_branch(&name, git2::BranchType::Local).ok()?;
    Some(name)
}

/// agefile content → agefile mtime → HEAD authordate (docs/DECISIONS.md #5).
fn last_modified(repo: &Repository) -> Option<String> {
    let agefile = repo.path().join("info/web/last-modified");
    if agefile.exists() {
        let from_content = fs::read_to_string(&agefile)
            .ok()
            .and_then(|content| parse_timestamp(content.trim()));
        return from_content
            .or_else(|| agefile_mtime(&agefile))
            .map(|zoned| format_rfc3339(&zoned));
    }
    head_authordate(repo).map(|zoned| format_rfc3339(&zoned))
}

/// Accepts RFC 3339 and the `git for-each-ref`-style date the post-receive
/// hook writes (`2026-07-24 13:06:00 +0900`), preserving the UTC offset.
fn parse_timestamp(s: &str) -> Option<Zoned> {
    for format in ["%Y-%m-%dT%H:%M:%S%:z", "%Y-%m-%d %H:%M:%S %z"] {
        if let Ok(zoned) = strtime::parse(format, s).and_then(|tm| tm.to_zoned()) {
            return Some(zoned);
        }
    }
    // Catch-all for remaining RFC 3339 shapes (fractional seconds, `Z`).
    s.parse::<Timestamp>()
        .ok()
        .map(|ts| ts.to_zoned(TimeZone::UTC))
}

fn agefile_mtime(path: &std::path::Path) -> Option<Zoned> {
    let mtime = fs::metadata(path).ok()?.modified().ok()?;
    let ts = Timestamp::try_from(mtime).ok()?;
    Some(ts.to_zoned(TimeZone::UTC))
}

fn head_authordate(repo: &Repository) -> Option<Zoned> {
    let commit = repo.head().ok()?.peel_to_commit().ok()?;
    git_time_to_zoned(commit.author().when())
}

/// Converts a git2 timestamp to `Zoned`, preserving the recorded UTC offset.
pub(crate) fn git_time_to_zoned(when: git2::Time) -> Option<Zoned> {
    let ts = Timestamp::from_second(when.seconds()).ok()?;
    let offset = Offset::from_seconds(when.offset_minutes() * 60).ok()?;
    Some(ts.to_zoned(TimeZone::fixed(offset)))
}

pub(crate) fn format_rfc3339(zoned: &Zoned) -> String {
    zoned.strftime(RFC3339_OUT).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_http_url_should_accept_only_http_and_https() {
        assert!(is_http_url("https://example.com"));
        assert!(is_http_url("http://example.com"));
        assert!(!is_http_url("javascript:alert(1)"));
        assert!(!is_http_url("ftp://example.com"));
        assert!(!is_http_url(""));
    }

    fn bare_repo_with_flag(
        section: &str,
        key: &str,
        value: bool,
    ) -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().expect("failed to create tempdir");
        let repo = Repository::init_bare(dir.path()).expect("failed to init bare repo");
        repo.config()
            .and_then(|mut config| config.set_bool(&format!("{section}.{key}"), value))
            .expect("failed to set config flag");
        (dir, repo)
    }

    #[test]
    fn should_list_is_true_by_default() {
        let dir = tempfile::tempdir().expect("failed to create tempdir");
        let repo = Repository::init_bare(dir.path()).expect("failed to init bare repo");
        assert!(should_list(&repo));
        assert!(!is_ignored(&repo));
    }

    #[test]
    fn should_list_is_false_when_hide_is_set() {
        let (_dir, repo) = bare_repo_with_flag("cgit", "hide", true);
        assert!(!should_list(&repo));
        // hide, unlike ignore, doesn't block direct access.
        assert!(!is_ignored(&repo));
    }

    #[test]
    fn should_list_and_is_ignored_agree_when_ignore_is_set() {
        let (_dir, repo) = bare_repo_with_flag("cgit", "ignore", true);
        assert!(!should_list(&repo));
        assert!(is_ignored(&repo));
    }

    #[test]
    fn axgit_ignore_wins_over_cgit_hide() {
        let dir = tempfile::tempdir().expect("failed to create tempdir");
        let repo = Repository::init_bare(dir.path()).expect("failed to init bare repo");
        let mut config = repo.config().expect("failed to open config");
        config
            .set_bool("cgit.hide", true)
            .expect("failed to set cgit.hide");
        config
            .set_bool("axgit.hide", false)
            .expect("failed to set axgit.hide");
        // [axgit] wins (docs/DECISIONS.md #5), so the repo is listed again.
        assert!(should_list(&repo));
    }

    fn bare_repo() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().expect("failed to create tempdir");
        let repo = Repository::init_bare(dir.path()).expect("failed to init bare repo");
        (dir, repo)
    }

    #[test]
    fn read_repo_info_treats_blank_metadata_as_unset() {
        let (_dir, repo) = bare_repo();
        repo.config()
            .and_then(|mut config| {
                config.set_str("cgit.section", "")?;
                config.set_str("cgit.owner", "   ")?;
                config.set_str("cgit.desc", "")
            })
            .expect("failed to set config strings");

        let info = read_repo_info(&repo, "blank-meta");
        assert_eq!(info.section, None);
        assert_eq!(info.owner, None);
        assert_eq!(info.description, None);
    }

    fn set_str(repo: &Repository, key: &str, value: &str) {
        repo.config()
            .and_then(|mut config| config.set_str(key, value))
            .expect("failed to set config string");
    }

    fn snapshot(repo: &Repository) -> git2::Config {
        repo.config()
            .and_then(|mut config| config.snapshot())
            .expect("failed to snapshot config")
    }

    #[test]
    fn module_link_template_falls_back_through_all_four_keys() {
        let (_dir, repo) = bare_repo();
        // Nothing set at all.
        assert_eq!(module_link_template(&snapshot(&repo), "vendor/dep"), None);

        // Repo-wide cgit only.
        set_str(&repo, "cgit.module-link", "cgit-wide");
        assert_eq!(
            module_link_template(&snapshot(&repo), "vendor/dep").as_deref(),
            Some("cgit-wide")
        );

        // Repo-wide axgit beats repo-wide cgit.
        set_str(&repo, "axgit.module-link", "axgit-wide");
        assert_eq!(
            module_link_template(&snapshot(&repo), "vendor/dep").as_deref(),
            Some("axgit-wide")
        );

        // Per-path cgit beats repo-wide axgit — specificity before section
        // precedence.
        set_str(&repo, "cgit.vendor/dep.module-link", "cgit-path");
        assert_eq!(
            module_link_template(&snapshot(&repo), "vendor/dep").as_deref(),
            Some("cgit-path")
        );

        // Per-path axgit beats everything.
        set_str(&repo, "axgit.vendor/dep.module-link", "axgit-path");
        assert_eq!(
            module_link_template(&snapshot(&repo), "vendor/dep").as_deref(),
            Some("axgit-path")
        );

        // A different path is untouched by the per-path keys above and
        // still resolves to the repo-wide template.
        assert_eq!(
            module_link_template(&snapshot(&repo), "other/dep").as_deref(),
            Some("axgit-wide")
        );
    }

    #[test]
    fn module_link_template_handles_a_dotted_path() {
        let (_dir, repo) = bare_repo();
        set_str(&repo, "axgit.vendor/lib.js.module-link", "dotted");
        assert_eq!(
            module_link_template(&snapshot(&repo), "vendor/lib.js").as_deref(),
            Some("dotted")
        );
    }

    #[test]
    fn module_link_template_empty_per_path_value_suppresses_repo_wide() {
        let (_dir, repo) = bare_repo();
        set_str(&repo, "axgit.module-link", "axgit-wide");
        set_str(&repo, "axgit.vendor/dep.module-link", "");
        // The per-path key wins even though its value is empty — the caller
        // (submodule::fill_module_links) treats that as explicit suppression
        // rather than falling through to the repo-wide template.
        assert_eq!(
            module_link_template(&snapshot(&repo), "vendor/dep").as_deref(),
            Some("")
        );
    }
}
