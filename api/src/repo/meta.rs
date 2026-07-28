use std::fs;

use git2::Repository;
use jiff::fmt::strtime;
use jiff::tz::{Offset, TimeZone};
use jiff::{Timestamp, Zoned};

use super::RepoInfo;

const RFC3339_OUT: &str = "%Y-%m-%dT%H:%M:%S%:z";

/// Reads list metadata for one repository. `name` is the directory name
/// without the `.git` suffix.
pub fn read_repo_info(repo: &Repository, name: &str) -> RepoInfo {
    // git2 requires a snapshot to read string values from a live config.
    let config = repo.config().and_then(|mut cfg| cfg.snapshot()).ok();
    let meta = |key: &str| config.as_ref().and_then(|cfg| config_value(cfg, key));

    RepoInfo {
        name: name.to_owned(),
        section: meta("section"),
        owner: meta("owner"),
        description: meta("desc"),
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

fn default_branch(repo: &Repository) -> Option<String> {
    repo.head().ok()?.shorthand().map(str::to_owned)
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
