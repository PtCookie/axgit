/**
 * Reserved `getStaticPaths` param the `/{repo}` page shells are built under.
 * The static build has one HTML file per *route shape*, not per repository —
 * the repository list is per-deployment and unknown at build time — so the
 * server rewrites `/{repo}/…` requests onto these files
 * (`api/src/shell.rs`, docs/DECISIONS.md #17).
 */
export const REPO_SHELL_PARAM = "__repo__";

/**
 * Maps a request path to the page shell that serves it, as a site-root path.
 * Mirrors `api/src/shell.rs::shell_for` — the two must change together; both
 * are covered by the same case table (`web/tests/lib/shell.test.ts`,
 * `api/src/shell.rs`'s unit tests).
 */
export function shellFor(pathname: string): string {
  const segments = pathname.split("/").filter((segment) => segment.length > 0);

  if (segments.length === 0) return "/";
  if (segments.length === 1) return `/${REPO_SHELL_PARAM}`;
  if (segments.length === 2 && segments[1] === "refs") return `/${REPO_SHELL_PARAM}/refs`;
  if (segments.length === 2 && segments[1] === "log") return `/${REPO_SHELL_PARAM}/log`;
  if (segments.length === 2 && segments[1] === "search") return `/${REPO_SHELL_PARAM}/search`;
  if (segments.length === 3 && segments[1] === "commit") return `/${REPO_SHELL_PARAM}/commit`;
  // tree: the path after `/tree/` is optional (empty means the root tree).
  if (segments.length >= 2 && segments[1] === "tree") return `/${REPO_SHELL_PARAM}/tree`;
  // blob: at least one path segment is required — there's nothing to show
  // for `/{repo}/blob` itself.
  if (segments.length >= 3 && segments[1] === "blob") return `/${REPO_SHELL_PARAM}/blob`;
  // blame: same "at least one path segment" rule as blob.
  if (segments.length >= 3 && segments[1] === "blame") return `/${REPO_SHELL_PARAM}/blame`;
  return "/404";
}
