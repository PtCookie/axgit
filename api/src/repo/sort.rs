//! Repository-index sort order (`?sort=` on `GET /api/v1/repos`,
//! `AXGIT_REPOSITORY_SORT`) — cgit's `s=name|desc|owner|idle|section` /
//! `repository-sort=age|name` (docs/DECISIONS.md #64).
//!
//! Sorting is applied in the handler, on a clone of the [`super::RepoInfo`]
//! snapshot `ScanCache` hands back — the snapshot itself stays name-ordered
//! (`scan.rs`), so it can be shared and sorted per request without a second
//! scan.

use std::cmp::Ordering;
use std::str::FromStr;

use serde::Serialize;
use utoipa::ToSchema;

use super::RepoInfo;

/// Sortable column. `?sort=` never adopts cgit's `s=` spelling (docs/
/// DECISIONS.md #61's precedent against reusing cgit's `h=`) — this enum's
/// `Serialize` impl is only used to echo the effective order back in
/// `ReposResponse.sort`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum RepoSort {
    Name,
    Desc,
    Owner,
    /// Last activity. Descending by default (`RepoOrder::reverse`'s starting
    /// value) — matches cgit's own `idle` sort, unlike every other key which
    /// defaults ascending.
    Idle,
    Section,
}

impl RepoSort {
    fn key(self, repo: &RepoInfo) -> Option<&str> {
        match self {
            Self::Name => Some(repo.name.as_str()),
            Self::Desc => repo.description.as_deref(),
            Self::Owner => repo.owner.as_deref(),
            Self::Idle => repo.last_modified.as_deref(),
            Self::Section => repo.section.as_deref(),
        }
    }
}

/// Unknown `?sort=`/`AXGIT_REPOSITORY_SORT` text. Carries no detail — every
/// caller (the query-param handler, the CLI `value_parser`) already has the
/// raw string in hand and builds its own message from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseRepoOrderError;

impl FromStr for RepoSort {
    type Err = ParseRepoOrderError;

    fn from_str(s: &str) -> Result<Self, ParseRepoOrderError> {
        match s {
            "name" => Ok(Self::Name),
            "desc" => Ok(Self::Desc),
            "owner" => Ok(Self::Owner),
            "idle" => Ok(Self::Idle),
            "section" => Ok(Self::Section),
            _ => Err(ParseRepoOrderError),
        }
    }
}

/// A parsed `?sort=` value: a column plus direction. `Display`s back to the
/// same spelling it was parsed from, for `ReposResponse.sort`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepoOrder {
    pub key: RepoSort,
    /// `true` sorts descending. `RepoSort::Idle` starts `true`; every other
    /// key starts `false`. A leading `-` in the raw string flips this once
    /// more — so `-idle` is ascending idle (oldest first), matching the
    /// "`-` always flips the key's own default" rule rather than hardcoding
    /// "idle is always descending".
    pub reverse: bool,
}

impl RepoOrder {
    /// Parses a raw `?sort=`/`AXGIT_REPOSITORY_SORT` value. `None` maps the
    /// empty/absent case to the default; unknown text is the caller's `400
    /// invalid_param` to raise.
    pub fn parse(raw: &str) -> Result<Self, ParseRepoOrderError> {
        let (negated, rest) = match raw.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, raw),
        };
        let key: RepoSort = rest.parse()?;
        let default_reverse = key == RepoSort::Idle;
        Ok(Self {
            key,
            reverse: default_reverse ^ negated,
        })
    }
}

impl std::fmt::Display for RepoOrder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self.key {
            RepoSort::Name => "name",
            RepoSort::Desc => "desc",
            RepoSort::Owner => "owner",
            RepoSort::Idle => "idle",
            RepoSort::Section => "section",
        };
        let default_reverse = self.key == RepoSort::Idle;
        if self.reverse != default_reverse {
            write!(f, "-{name}")
        } else {
            write!(f, "{name}")
        }
    }
}

impl Default for RepoOrder {
    /// `name` ascending — today's fixed order, unchanged when `?sort=` and
    /// `AXGIT_REPOSITORY_SORT` are both absent.
    fn default() -> Self {
        Self {
            key: RepoSort::Name,
            reverse: false,
        }
    }
}

/// Sorts `repos` in place by `order`. `None` values always sort last
/// regardless of direction — a repo missing a description shouldn't jump to
/// the top under `-desc` — with ties (including all-`None` groups) broken by
/// `name` ascending, always, so the order is stable and never depends on the
/// snapshot's incoming order.
///
/// `RepoSort::Idle` compares parsed timestamps, not the formatted strings —
/// `meta::format_rfc3339` preserves each commit's own UTC offset, so string
/// order would be wrong whenever two repos' agefiles carry different offsets.
pub fn sort_repos(repos: &mut [RepoInfo], order: RepoOrder) {
    repos.sort_by(|a, b| {
        let cmp = match order.key {
            RepoSort::Idle => compare_idle(a, b, order.reverse),
            key => compare_opt_str(key.key(a), key.key(b), order.reverse),
        };
        cmp.then_with(|| a.name.cmp(&b.name))
    });
}

/// `None` always sorts last: `reverse` only flips the comparison between two
/// present values, never the `None`-vs-`Some` branches, so a repo missing a
/// field can't jump to the top just because the direction was reversed.
fn compare_opt_str(a: Option<&str>, b: Option<&str>, reverse: bool) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => {
            let cmp = a.cmp(b);
            if reverse { cmp.reverse() } else { cmp }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn compare_idle(a: &RepoInfo, b: &RepoInfo, reverse: bool) -> Ordering {
    let parse = |repo: &RepoInfo| {
        repo.last_modified
            .as_deref()
            .and_then(|s| s.parse::<jiff::Timestamp>().ok())
    };
    match (parse(a), parse(b)) {
        (Some(a), Some(b)) => {
            let cmp = a.cmp(&b);
            if reverse { cmp.reverse() } else { cmp }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(name: &str, desc: Option<&str>, last_modified: Option<&str>) -> RepoInfo {
        RepoInfo {
            name: name.to_owned(),
            section: None,
            owner: None,
            description: desc.map(str::to_owned),
            homepage: None,
            default_branch: None,
            last_modified: last_modified.map(str::to_owned),
        }
    }

    #[test]
    fn parse_should_default_each_key_ascending_except_idle() {
        assert_eq!(
            RepoOrder::parse("name").unwrap(),
            RepoOrder {
                key: RepoSort::Name,
                reverse: false
            }
        );
        assert_eq!(
            RepoOrder::parse("idle").unwrap(),
            RepoOrder {
                key: RepoSort::Idle,
                reverse: true
            }
        );
    }

    #[test]
    fn parse_should_flip_the_key_default_on_a_leading_dash() {
        assert_eq!(
            RepoOrder::parse("-name").unwrap(),
            RepoOrder {
                key: RepoSort::Name,
                reverse: true
            }
        );
        assert_eq!(
            RepoOrder::parse("-idle").unwrap(),
            RepoOrder {
                key: RepoSort::Idle,
                reverse: false
            }
        );
    }

    #[test]
    fn parse_should_reject_unknown_keys() {
        assert!(RepoOrder::parse("bogus").is_err());
        assert!(RepoOrder::parse("-").is_err());
    }

    #[test]
    fn display_should_round_trip_parse() {
        for raw in ["name", "-name", "idle", "-idle", "desc", "-desc"] {
            assert_eq!(RepoOrder::parse(raw).unwrap().to_string(), raw);
        }
    }

    #[test]
    fn sort_repos_should_place_none_last_regardless_of_direction() {
        let mut repos = vec![
            repo("b", Some("beta"), None),
            repo("a", None, None),
            repo("c", Some("alpha"), None),
        ];
        sort_repos(
            &mut repos,
            RepoOrder {
                key: RepoSort::Desc,
                reverse: false,
            },
        );
        assert_eq!(
            repos.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["c", "b", "a"]
        );

        sort_repos(
            &mut repos,
            RepoOrder {
                key: RepoSort::Desc,
                reverse: true,
            },
        );
        assert_eq!(
            repos.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["b", "c", "a"]
        );
    }

    #[test]
    fn sort_repos_should_break_ties_by_name_ascending() {
        let mut repos = vec![repo("b", None, None), repo("a", None, None)];
        sort_repos(&mut repos, RepoOrder::default());
        assert_eq!(
            repos.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
    }

    #[test]
    fn sort_repos_should_compare_idle_by_parsed_timestamp_not_string() {
        // Two offsets where the formatted string order disagrees with real
        // time order: "+0900" sorts after "-0500" lexicographically, but the
        // instant it names is earlier.
        let mut repos = vec![
            repo("early", None, Some("2026-07-24T13:06:00+09:00")),
            repo("late", None, Some("2026-07-24T05:00:00-05:00")),
        ];
        sort_repos(
            &mut repos,
            RepoOrder {
                key: RepoSort::Idle,
                reverse: false,
            },
        );
        assert_eq!(
            repos.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["early", "late"]
        );
    }
}
