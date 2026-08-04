import { encodePath, encodeSegment } from "@/lib/api/path";
import { DEFAULT_CONTEXT, type DiffOptions, diffOptionsQuery } from "@/lib/diff-options";

/** Appends `?ref=` when a non-default ref is selected — shared by every
 *  href builder below (`?ref=`-only ref selection, DECISIONS.md #18). */
function refQuery(ref: string | undefined): string {
  return ref ? `?ref=${encodeSegment(ref)}` : "";
}

/** Builds a `/{repo}/tree/{path}` href, shared with `TreeView` (the parent-
 *  directory row uses the same link shape as a breadcrumb ancestor) and
 *  `PathBreadcrumbs`. */
export function treeHref(repo: string, path: string, ref: string | undefined): string {
  return `/${encodeSegment(repo)}/tree${path ? `/${encodePath(path)}` : ""}${refQuery(ref)}`;
}

/** Builds a `/{repo}/blob/{path}` href. `path` must be non-empty — there's
 *  nothing to show for `/{repo}/blob` itself. */
export function blobHref(repo: string, path: string, ref: string | undefined): string {
  return `/${encodeSegment(repo)}/blob/${encodePath(path)}${refQuery(ref)}`;
}

/** Builds a `/{repo}/refs` href. Takes no `ref` param — the refs page *is*
 *  the ref listing, so unlike `treeHref`/`blobHref` there is no `?ref=`. */
export function refsHref(repo: string): string {
  return `/${encodeSegment(repo)}/refs`;
}

/** Builds a `/{repo}/log` href from the given params, omitting any left
 *  unset — same shape as `searchHref`/`statsHref`. Moved out of
 *  `CommitLog.tsx` (docs/DECISIONS.md #34) so `RefBadges`' ref links can
 *  share it too. */
export function logHref(repo: string, params: { ref?: string; path?: string; cursor?: string } = {}): string {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value) search.set(key, value);
  }
  const query = search.toString();
  return `/${encodeSegment(repo)}/log${query ? `?${query}` : ""}`;
}

/** Builds a `/{repo}/commit/{sha}` href, optionally carrying the current diff
 *  display options forward — so following a parent-commit link keeps the
 *  same context/ignore-whitespace choice instead of silently resetting it.
 *  `view` isn't accepted: unified vs. split isn't meaningful to carry across
 *  a navigation to a *different* commit's diff the way context/ignorews are. */
export function commitHref(
  repo: string,
  sha: string,
  options: Partial<Pick<DiffOptions, "context" | "ignorews">> = {},
): string {
  const query = diffOptionsQuery({
    view: "unified",
    context: options.context ?? DEFAULT_CONTEXT,
    ignorews: options.ignorews ?? false,
  });
  const search = new URLSearchParams(query).toString();
  return `/${encodeSegment(repo)}/commit/${encodeSegment(sha)}${search ? `?${search}` : ""}`;
}

/** Builds a `/{repo}/blame/{path}` href. Same non-empty-`path` rule as
 *  `blobHref`. */
export function blameHref(repo: string, path: string, ref: string | undefined): string {
  return `/${encodeSegment(repo)}/blame/${encodePath(path)}${refQuery(ref)}`;
}

/** Builds a `/{repo}/blob/{path}` href pointing at one matched line
 *  (`#L{n}`, `CodeBlock.tsx`'s existing line-anchor scheme) — used by
 *  `SearchView`'s content-match rows. */
export function blobLineHref(repo: string, path: string, ref: string | undefined, line: number): string {
  return `${blobHref(repo, path, ref)}#L${line}`;
}

/** Builds a `/{repo}/search` href from the given params, omitting any left
 *  unset. Mirrors `lib/api/repos.ts::searchRepo`'s param shape (`q`/`type`/
 *  `ref`), but stays a plain query-string builder so `SearchView`'s
 *  `<form method="get">` and its "clear filter"-style links can share it. */
export function searchHref(repo: string, params: { q?: string; type?: string; ref?: string } = {}): string {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value) search.set(key, value);
  }
  const query = search.toString();
  return `/${encodeSegment(repo)}/search${query ? `?${query}` : ""}`;
}

/** Builds a `/{repo}/stats` href from the given params, omitting any left
 *  unset. Same shape as `searchHref` — used for the four fixed period links
 *  (`StatsView`), which are plain anchors rather than a form since the
 *  choice is a fixed 4-way pick, not free text. */
export function statsHref(repo: string, params: { period?: string; ref?: string } = {}): string {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value) search.set(key, value);
  }
  const query = search.toString();
  return `/${encodeSegment(repo)}/stats${query ? `?${query}` : ""}`;
}
