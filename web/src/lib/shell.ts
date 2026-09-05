import { REPO_SHELL_PARAM, SHELL_ROUTES } from "./shell-routes";

/**
 * Re-exported so the `.astro` pages (and anything else) keep importing the
 * placeholder param from `lib/shell` — the route *table* moved to
 * `lib/shell-routes.ts` (docs/DECISIONS.md #88), the entry point did not.
 */
export { REPO_SHELL_PARAM };

/**
 * Maps a request path to the page shell that serves it, as a site-root path.
 *
 * Walks the same table (`lib/shell-routes.ts`) the build emits to
 * `dist/shell-routes.json` and `api/src/shell.rs::shell_for` reads, so the
 * dev server and the production server agree by construction rather than by
 * two matchers being kept in sync by hand (docs/DECISIONS.md #88).
 */
export function shellFor(pathname: string): string {
  const segments = pathname.split("/").filter((segment) => segment.length > 0);

  if (segments.length === 0) return "/";

  // Only the *shape* past the repository segment decides the shell; the
  // repository name itself is never inspected.
  const rest = segments.slice(1);

  for (const route of SHELL_ROUTES) {
    if (route.segment === null) {
      if (rest.length === 0) return `/${REPO_SHELL_PARAM}`;
      continue;
    }
    if (rest[0] !== route.segment) continue;
    const extra = rest.length - 1;
    if (extra < route.minExtra) continue;
    if (route.maxExtra !== null && extra > route.maxExtra) continue;
    return `/${REPO_SHELL_PARAM}/${route.shell}`;
  }

  return "/404";
}
