//! Commit-activity statistics: period-bucketed commit counts plus a
//! per-author breakdown (`GET /api/v1/repos/{repo}/stats`, docs/API.md).
//! cgit's `stats` page — the last gap versus cgit (docs/DECISIONS.md #9's
//! other v1 exclusion, closed by #28).
//!
//! git2 in-process revwalk, same reasoning as search (#26): an index would be
//! this app's first piece of mutable, persistent state, and a `git log` exec
//! could only be bounded by a process timeout, whereas git2 lets the walk
//! enforce an exact commit budget.

use std::collections::HashMap;

use git2::{Commit, Repository};
use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{ToSpan, Zoned};
use serde::Serialize;
use utoipa::ToSchema;

use super::commits::{self, CommitAuthor};
use super::meta;
use crate::error::ApiError;

/// Bucket count returned regardless of `period` — a fixed window keeps the
/// response shape (and the web table's width) independent of the selection.
const BUCKET_COUNT: usize = 12;

/// Commits walked before giving up, independent of the bucket window — same
/// budget family as search's `MAX_SCANNED_COMMITS` (`repo/search.rs`).
const MAX_SCANNED_COMMITS: usize = 20_000;

/// Requested bucket size, echoed back in the response's `period` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum StatsPeriod {
    Week,
    Month,
    Quarter,
    Year,
}

/// One time bucket (docs/API.md).
#[derive(Debug, Serialize, ToSchema)]
pub struct BucketStats {
    /// Bucket start, UTC, RFC 3339. Ascending order (oldest first).
    pub start: String,
    /// Total commits in this bucket, including any authors cut by `limit`.
    pub commits: usize,
}

/// Per-author breakdown (docs/API.md).
#[derive(Debug, Serialize, ToSchema)]
pub struct AuthorStats {
    pub author: CommitAuthor,
    /// Total commits by this author within the window.
    pub commits: usize,
    /// Parallel to the response's `buckets` — same length and order.
    pub buckets: Vec<usize>,
}

/// Response of `GET /api/v1/repos/{repo}/stats` (docs/API.md).
#[derive(Debug, Serialize, ToSchema)]
pub struct StatsResults {
    /// Resolved commit sha the window is anchored on. `None` only for an
    /// empty repository (unborn HEAD) with no explicit `ref`.
    #[schema(required = true)]
    pub sha: Option<String>,
    pub period: StatsPeriod,
    /// `true` when the commit scan budget was hit, or the author list was cut
    /// by `limit` — either way, the results are a prefix.
    pub truncated: bool,
    /// Number of distinct authors within the window (may exceed
    /// `authors.len()` once cut by `limit`).
    pub author_count: usize,
    pub buckets: Vec<BucketStats>,
    /// Sorted by commit count, descending; capped at `limit`.
    pub authors: Vec<AuthorStats>,
}

/// Runs the stats aggregation for `period`, anchored on `commit`'s
/// authordate, keeping at most `limit` authors.
pub fn stats(
    repo: &Repository,
    commit: &Commit,
    period: StatsPeriod,
    limit: usize,
) -> Result<StatsResults, ApiError> {
    // Falls back to now() in the (extremely unlikely) case the commit's
    // authordate can't convert (`meta::git_time_to_zoned` — the same guard
    // `authored_at` applies elsewhere) — the window still needs an anchor.
    let anchor = meta::git_time_to_zoned(commit.author().when()).unwrap_or_else(Zoned::now);
    let starts = bucket_starts(&anchor, period)?;
    let window_end = starts[BUCKET_COUNT - 1].checked_add(period_span(period, 1))?;

    let mut revwalk = repo.revwalk()?;
    revwalk.push(commit.id())?;
    let mut bucket_totals = vec![0usize; BUCKET_COUNT];
    let mut authors: HashMap<String, (CommitAuthor, Vec<usize>)> = HashMap::new();
    let mut truncated = false;
    for (scanned, oid) in revwalk.enumerate() {
        if scanned >= MAX_SCANNED_COMMITS {
            truncated = true;
            break;
        }
        let oid = oid?;
        let found = repo.find_commit(oid)?;
        let Some(when) = meta::git_time_to_zoned(found.author().when()) else {
            continue; // unrepresentable timestamp — same skip as authored_at: None elsewhere
        };
        let Some(index) = bucket_index(when.timestamp(), &starts, &window_end) else {
            continue; // older than the BUCKET_COUNT-period window
        };
        bucket_totals[index] += 1;
        let author = commits::signature_info(&found.author());
        let entry = authors
            .entry(author.email_hash.clone())
            .or_insert_with(|| (author, vec![0usize; BUCKET_COUNT]));
        entry.1[index] += 1;
    }

    let author_count = authors.len();
    let mut authors: Vec<AuthorStats> = authors
        .into_values()
        .map(|(author, buckets)| AuthorStats {
            commits: buckets.iter().sum(),
            author,
            buckets,
        })
        .collect();
    // Deterministic order: commit count descending, then hash. Ties are rare
    // (small windows), but a stable order matters for repeatable tests and a
    // UI that shouldn't jitter across identical requests.
    authors.sort_by(|a, b| {
        b.commits
            .cmp(&a.commits)
            .then_with(|| a.author.email_hash.cmp(&b.author.email_hash))
    });
    if authors.len() > limit {
        authors.truncate(limit);
        truncated = true;
    }

    let buckets = starts
        .iter()
        .zip(bucket_totals)
        .map(|(start, commits)| BucketStats {
            start: meta::format_rfc3339(start),
            commits,
        })
        .collect();

    Ok(StatsResults {
        sha: Some(commit.id().to_string()),
        period,
        truncated,
        author_count,
        buckets,
        authors,
    })
}

/// The `BUCKET_COUNT` ascending bucket starts (UTC) for `period`, the last
/// one being the period containing `anchor`.
fn bucket_starts(anchor: &Zoned, period: StatsPeriod) -> Result<Vec<Zoned>, ApiError> {
    let anchor_date = anchor.timestamp().to_zoned(TimeZone::UTC).date();
    let last_start = period_start_date(anchor_date, period)?;
    let mut starts = Vec::with_capacity(BUCKET_COUNT);
    for back in (0..BUCKET_COUNT).rev() {
        let date = last_start.checked_sub(period_span(period, back as i64))?;
        starts.push(date.to_zoned(TimeZone::UTC)?);
    }
    Ok(starts)
}

/// The first instant of the `period` that `date` falls in — the Monday of
/// its week, the 1st of its month/quarter's first month, or Jan 1 of its
/// year.
fn period_start_date(date: Date, period: StatsPeriod) -> Result<Date, ApiError> {
    Ok(match period {
        StatsPeriod::Week => {
            let monday_offset = i64::from(date.weekday().to_monday_zero_offset());
            date.checked_sub(monday_offset.days())?
        }
        StatsPeriod::Month => date.first_of_month(),
        StatsPeriod::Quarter => {
            let quarter_start_month = (date.month() - 1) / 3 * 3 + 1;
            Date::new(date.year(), quarter_start_month, 1)?
        }
        StatsPeriod::Year => Date::new(date.year(), 1, 1)?,
    })
}

/// A span of `count` periods, for calendar-aware `Date`/`Zoned` arithmetic
/// (month/quarter/year lengths vary; jiff's `Span` accounts for that).
fn period_span(period: StatsPeriod, count: i64) -> jiff::Span {
    match period {
        StatsPeriod::Week => count.weeks(),
        StatsPeriod::Month => count.months(),
        StatsPeriod::Quarter => (count * 3).months(),
        StatsPeriod::Year => count.years(),
    }
}

/// The bucket `instant` falls into, or `None` when it's older than the
/// window's first bucket. An `instant` at or past `window_end` (a commit
/// authored later than the anchor — possible with out-of-order authordates
/// on merged branches) clamps into the last bucket rather than being dropped.
fn bucket_index(instant: jiff::Timestamp, starts: &[Zoned], window_end: &Zoned) -> Option<usize> {
    if instant < starts[0].timestamp() {
        return None;
    }
    if instant >= window_end.timestamp() {
        return Some(starts.len() - 1);
    }
    match starts.binary_search_by(|start| start.timestamp().cmp(&instant)) {
        Ok(index) => Some(index),
        // `instant >= starts[0]` is already established above, so `index`
        // (the insertion point) is at least 1: the previous start is the
        // bucket `instant` falls into.
        Err(index) => Some(index - 1),
    }
}

#[cfg(test)]
mod tests {
    use git2::{Oid, Signature, Time};

    use super::*;

    /// Builds a `Signature` with a controlled author/committer date (unlike
    /// `blame.rs`/`search.rs`'s sibling helper, which hardcodes
    /// `Signature::now`) — `when` is an RFC 3339 instant.
    fn signature_at(name: &str, email: &str, when: &str) -> Signature<'static> {
        let instant = when.parse::<jiff::Timestamp>().unwrap();
        Signature::new(name, email, &Time::new(instant.as_second(), 0)).unwrap()
    }

    /// Commits an empty tree on top of `parent` (stats only reads commit
    /// metadata, not file content) with a controlled author date.
    fn commit_at(repo: &Repository, parent: Option<Oid>, when: &str, message: &str) -> Oid {
        let builder = repo.treebuilder(None).unwrap();
        let tree = repo.find_tree(builder.write().unwrap()).unwrap();
        let sig = signature_at("Test", "test@example.com", when);
        let parents: Vec<_> = parent
            .map(|oid| repo.find_commit(oid).unwrap())
            .into_iter()
            .collect();
        let parent_refs: Vec<_> = parents.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parent_refs)
            .unwrap()
    }

    #[test]
    fn period_start_date_should_align_to_the_expected_boundary() {
        // 2026-07-24 is a Friday.
        let date = "2026-07-24".parse::<Date>().unwrap();
        assert_eq!(
            period_start_date(date, StatsPeriod::Week).unwrap(),
            "2026-07-20".parse::<Date>().unwrap()
        );
        assert_eq!(
            period_start_date(date, StatsPeriod::Month).unwrap(),
            "2026-07-01".parse::<Date>().unwrap()
        );
        assert_eq!(
            period_start_date(date, StatsPeriod::Quarter).unwrap(),
            "2026-07-01".parse::<Date>().unwrap()
        );
        assert_eq!(
            period_start_date(date, StatsPeriod::Year).unwrap(),
            "2026-01-01".parse::<Date>().unwrap()
        );
    }

    #[test]
    fn bucket_starts_should_produce_twelve_ascending_boundaries_ending_at_the_anchor_period() {
        let anchor = "2026-07-24T12:00:00Z"
            .parse::<jiff::Timestamp>()
            .unwrap()
            .to_zoned(TimeZone::UTC);
        let starts = bucket_starts(&anchor, StatsPeriod::Month).unwrap();
        assert_eq!(starts.len(), BUCKET_COUNT);
        assert!(starts.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(starts[11].date(), "2026-07-01".parse::<Date>().unwrap());
        assert_eq!(starts[0].date(), "2025-08-01".parse::<Date>().unwrap());
    }

    #[test]
    fn stats_should_aggregate_commits_into_buckets_and_authors() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();

        let c0 = commit_at(&repo, None, "2026-05-01T00:00:00Z", "first");
        let c1 = commit_at(&repo, Some(c0), "2026-06-01T00:00:00Z", "second");
        let c2 = commit_at(&repo, Some(c1), "2026-07-01T00:00:00Z", "third");
        let commit = repo.find_commit(c2).unwrap();

        let results = stats(&repo, &commit, StatsPeriod::Month, 50).unwrap();
        assert_eq!(results.sha, Some(c2.to_string()));
        assert_eq!(results.period, StatsPeriod::Month);
        assert!(!results.truncated);
        assert_eq!(results.buckets.len(), BUCKET_COUNT);
        // All three commits share the same author (`commit_at` hardcodes
        // "Test" / "test@example.com"), so exactly one bucket per commit
        // month should read 1 and the rest 0.
        let nonzero: Vec<usize> = results
            .buckets
            .iter()
            .filter(|bucket| bucket.commits > 0)
            .map(|bucket| bucket.commits)
            .collect();
        assert_eq!(nonzero, vec![1, 1, 1]);
        assert_eq!(results.author_count, 1);
        assert_eq!(results.authors.len(), 1);
        assert_eq!(results.authors[0].commits, 3);
        assert_eq!(
            results.authors[0].buckets.iter().sum::<usize>(),
            results.authors[0].commits
        );
    }

    #[test]
    fn stats_should_drop_commits_older_than_the_window() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let old = commit_at(&repo, None, "2000-01-01T00:00:00Z", "ancient");
        let recent = commit_at(&repo, Some(old), "2026-07-01T00:00:00Z", "recent");
        let commit = repo.find_commit(recent).unwrap();

        let results = stats(&repo, &commit, StatsPeriod::Month, 50).unwrap();
        let total: usize = results.buckets.iter().map(|bucket| bucket.commits).sum();
        assert_eq!(
            total, 1,
            "the year-2000 commit falls outside the 12-month window"
        );
    }

    #[test]
    fn stats_should_truncate_the_author_list_by_limit_but_keep_full_bucket_totals() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let builder = repo.treebuilder(None).unwrap();
        let tree = repo.find_tree(builder.write().unwrap()).unwrap();
        let mut parent: Option<Oid> = None;
        for name in ["alice", "bob", "carol"] {
            let sig = signature_at(name, &format!("{name}@example.com"), "2026-07-01T00:00:00Z");
            let parents: Vec<_> = parent
                .map(|oid| repo.find_commit(oid).unwrap())
                .into_iter()
                .collect();
            let parent_refs: Vec<_> = parents.iter().collect();
            let oid = repo
                .commit(Some("HEAD"), &sig, &sig, "msg", &tree, &parent_refs)
                .unwrap();
            parent = Some(oid);
        }
        let commit = repo.find_commit(parent.unwrap()).unwrap();

        let results = stats(&repo, &commit, StatsPeriod::Month, 2).unwrap();
        assert_eq!(results.author_count, 3);
        assert_eq!(results.authors.len(), 2);
        assert!(results.truncated);
        let bucket_total: usize = results.buckets.iter().map(|bucket| bucket.commits).sum();
        assert_eq!(
            bucket_total, 3,
            "bucket totals include the truncated-out author"
        );
    }
}
