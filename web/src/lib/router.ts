import { encodeSegment } from "@/lib/api/path";

/**
 * Client-side route, derived from `location.pathname`. There is no history
 * router (DECISIONS.md #16) — every navigation is a full page load, so
 * `parseRoute` only ever needs to run once per page against the URL the
 * browser already fetched.
 */
export type Route =
  { name: "repos" } | { name: "repo"; repo: string } | { name: "refs"; repo: string } | { name: "not-found" };

export function parseRoute(pathname: string): Route {
  const segments = pathname
    .split("/")
    .filter((segment) => segment.length > 0)
    .map(decodeURIComponent);

  if (segments.length === 0) {
    return { name: "repos" };
  }

  const [repo, sub, ...rest] = segments;
  if (rest.length > 0) {
    return { name: "not-found" };
  }
  if (sub === undefined) {
    return { name: "repo", repo };
  }
  if (sub === "refs") {
    return { name: "refs", repo };
  }
  return { name: "not-found" };
}

/** Builds a link to a repository page (or one of its sub-pages). */
export function repoUrl(repo: string, sub?: string): string {
  const base = `/${encodeSegment(repo)}`;
  return sub ? `${base}/${sub}` : base;
}
