/**
 * The route-shape table the prerendered page shells are matched against —
 * the single place a `/{repo}/…` route shape is declared (docs/DECISIONS.md
 * #88, refining #17).
 *
 * The static build has one HTML file per route *shape*, not per repository,
 * so both the `astro dev` middleware (`shellFor`) and the production server
 * (`api/src/shell.rs`) have to map a request path onto the right shell.
 * Rather than keeping that mapping in two hand-written matchers, this table
 * is emitted to `dist/shell-routes.json` by the build (`astro.config.mjs`'s
 * `shellRoutesManifest` integration) and the Rust side reads it from there.
 * Adding a route means adding a page under `src/pages/[repo]/` and an entry
 * here — the build fails if the two disagree.
 */

/**
 * Reserved `getStaticPaths` param the `/{repo}` page shells are built under.
 * The repository list is per-deployment and unknown at build time, so the
 * server rewrites `/{repo}/…` requests onto files built under this
 * placeholder instead (docs/DECISIONS.md #17).
 */
export const REPO_SHELL_PARAM = "__repo__";

export interface ShellRoute {
  /**
   * The literal segment right after `{repo}`, or `null` for the repository
   * index (`/{repo}` itself, which has no such segment).
   */
  segment: string | null;
  /**
   * Directory under {@link REPO_SHELL_PARAM} holding the built shell, or
   * `null` for the index shell. Always equal to `segment` today; kept
   * explicit so a future route whose URL segment differs from its page
   * directory doesn't need a schema change.
   */
  shell: string | null;
  /** Fewest segments allowed *after* `segment`. */
  minExtra: number;
  /** Most segments allowed after `segment`; `null` for an open rest param. */
  maxExtra: number | null;
}

/**
 * Matched in order, first hit wins. Segments are unique, so the order is not
 * load-bearing today — it is preserved verbatim into the manifest anyway so
 * both matchers behave identically if that ever stops being true.
 */
export const SHELL_ROUTES: readonly ShellRoute[] = [
  // `/{repo}` — the repository summary.
  { segment: null, shell: null, minExtra: 0, maxExtra: 0 },
  { segment: "refs", shell: "refs", minExtra: 0, maxExtra: 0 },
  { segment: "log", shell: "log", minExtra: 0, maxExtra: 0 },
  { segment: "search", shell: "search", minExtra: 0, maxExtra: 0 },
  { segment: "stats", shell: "stats", minExtra: 0, maxExtra: 0 },
  // The compare page never carries the revisions in the path (they're
  // `?from=`/`?to=` query params, since a ref may itself contain `/`) — a
  // bare 2-segment shape is the whole story, unlike commit's 3-segment one.
  { segment: "diff", shell: "diff", minExtra: 0, maxExtra: 0 },
  { segment: "commit", shell: "commit", minExtra: 1, maxExtra: 1 },
  // An oid is a single fixed segment, never containing `/` — same
  // exactly-one-extra-segment shape as commit.
  { segment: "object", shell: "object", minExtra: 1, maxExtra: 1 },
  // tree: the path after `/tree/` is optional (empty means the root tree).
  { segment: "tree", shell: "tree", minExtra: 0, maxExtra: null },
  // blob: at least one path segment is required — there's nothing to show
  // for `/{repo}/blob` itself.
  { segment: "blob", shell: "blob", minExtra: 1, maxExtra: null },
  // blame: same "at least one path segment" rule as blob.
  { segment: "blame", shell: "blame", minExtra: 1, maxExtra: null },
  // tag: at least one name segment is required (a tag name may itself
  // contain `/`) — there's nothing to show for `/{repo}/tag` itself, and the
  // refs page already is the tag listing.
  { segment: "tag", shell: "tag", minExtra: 1, maxExtra: null },
];

/**
 * The serialized form written to `dist/shell-routes.json` and read by
 * `api/src/shell.rs`. Field names are camelCase on both sides; the Rust
 * structs rename rather than the JSON accommodating them, so this file stays
 * ordinary TypeScript.
 */
export function shellRoutesManifest(): {
  repoShellParam: string;
  routes: readonly ShellRoute[];
} {
  return { repoShellParam: REPO_SHELL_PARAM, routes: SHELL_ROUTES };
}
