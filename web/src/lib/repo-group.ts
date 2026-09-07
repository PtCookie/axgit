import type { RepoInfo } from "@/lib/api/schemas";
import { idleTime } from "@/lib/repo-sort";

/**
 * Groups the repository index by category (`section`) for `RepoList.tsx`.
 * Kept out of the component — like `repo-filter.ts` and `repo-sort.ts` — so
 * the group-ordering rule is a pure function with its own unit tests.
 */

export interface RepoGroup {
  section: string | null;
  repos: RepoInfo[];
}

/** A category's own last activity: the most recent `last_modified` among its
 *  repositories, or `null` when none of them has one. */
function groupIdleTime(group: RepoGroup): number | null {
  let newest: number | null = null;
  for (const repo of group.repos) {
    const time = idleTime(repo);
    if (time !== null && (newest === null || time > newest)) {
      newest = time;
    }
  }
  return newest;
}

/** Groups `repos` by `section`, preserving each group's incoming repo order
 *  (whatever sort the caller already applied), and orders the groups:
 *  unsectioned first, then named categories most recently active first.
 *
 *  - section: null (unsectioned) always sorts first — those repos have no group of their own to
 *    land in, so surfacing them ahead of every named category keeps them from getting lost between
 *    the curated ones (docs/DECISIONS.md #86).
 *  - Named groups sort by their own last activity, descending (docs/DECISIONS.md #93) — a category
 *    is as recent as its most recently pushed repository. Group order never depends on the active
 *    column sort; `?sort=` only reorders rows *within* a group.
 *  - A category whose every repository lacks a `last_modified` sorts last among the named ones,
 *    mirroring `repo-sort.ts`'s nulls-last rule, and equal-recency ties break by section name
 *    ascending — plain code-point comparison, like `compareOptStr`, not `localeCompare`, to match
 *    the API's own `str::cmp`-based ordering (api/src/repo/sort.rs). */
export function groupBySection(repos: RepoInfo[]): RepoGroup[] {
  const groups: RepoGroup[] = [];

  for (const repo of repos) {
    const existing = groups.find((group) => group.section === repo.section);
    if (existing) {
      existing.repos.push(repo);
    } else {
      groups.push({ section: repo.section, repos: [repo] });
    }
  }

  const recency = new Map<RepoGroup, number | null>(groups.map((group) => [group, groupIdleTime(group)]));

  groups.sort((a, b) => {
    if (a.section === null) return b.section === null ? 0 : -1;
    if (b.section === null) return 1;

    const aTime = recency.get(a) ?? null;
    const bTime = recency.get(b) ?? null;
    if (aTime !== null && bTime !== null && aTime !== bTime) {
      return bTime - aTime;
    }
    if (aTime !== null && bTime === null) return -1;
    if (aTime === null && bTime !== null) return 1;

    return a.section < b.section ? -1 : a.section > b.section ? 1 : 0;
  });
  return groups;
}
