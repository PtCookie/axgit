import type { RepoInfo } from "@/lib/api/schemas";

/**
 * Client-side filter for the repository list (`RepoList.tsx`,
 * docs/DECISIONS.md #25). `GET /api/v1/repos` already returns every field
 * matched here, so this needs no API change — it's the first, "obviously in
 * bounds" cut of ROADMAP.md's repository search item. Searching *inside*
 * repository content (file contents, commit messages) is a separate,
 * server-side follow-up.
 */

/** Fields searched, in the order checked. */
const FIELDS: readonly (keyof RepoInfo)[] = ["name", "description", "owner", "section"];

/** True if `repo` matches every whitespace-separated term in `query`
 *  (AND), each term matched case-insensitively as a substring of any of
 *  `FIELDS`. An empty/whitespace-only query matches everything. */
function matches(repo: RepoInfo, terms: string[]): boolean {
  return terms.every((term) =>
    FIELDS.some((field) => {
      const value = repo[field];
      return typeof value === "string" && value.toLocaleLowerCase().includes(term);
    }),
  );
}

/** Filters `repos` by `query`. An empty/whitespace-only query returns
 *  `repos` unchanged (same array reference, so callers can cheaply detect
 *  "no filter active"). */
export function filterRepos(repos: RepoInfo[], query: string): RepoInfo[] {
  const trimmed = query.trim();
  if (trimmed === "") return repos;

  const terms = trimmed.toLocaleLowerCase().split(/\s+/);
  return repos.filter((repo) => matches(repo, terms));
}
