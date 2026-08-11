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

/** Builds a `/{repo}/tag/{name}` href. `encodePath`, not `encodeSegment` — a
 *  literal `/` inside `name` (e.g. `release/1.0`) must survive into the URL
 *  for `shellFor`'s tag arm to match and for `tagNameFromPathname` to
 *  rejoin it, same as `blobHref`'s path. No `?ref=`: the tag name *is* the
 *  ref, same reasoning as `refsHref`. */
export function tagHref(repo: string, name: string): string {
  return `/${encodeSegment(repo)}/tag/${encodePath(name)}`;
}

/** Builds a `/{repo}/object/{oid}` href. `encodeSegment`, not `encodePath` —
 *  unlike a tag name, an oid never contains `/`, same reasoning as
 *  `commitHref`. No `?ref=`: the oid *is* the address, same reasoning as
 *  `tagHref`. */
export function objectHref(repo: string, oid: string): string {
  return `/${encodeSegment(repo)}/object/${encodeSegment(oid)}`;
}

/** Builds a `/{repo}/log` href from the given params, omitting any left
 *  unset — same shape as `searchHref`/`statsHref`. Moved out of
 *  `CommitLog.tsx` (docs/DECISIONS.md #34) so `RefBadges`' ref links can
 *  share it too. */
export function logHref(
  repo: string,
  params: {
    ref?: string;
    path?: string;
    cursor?: string;
    msg?: string;
    follow?: string;
    stat?: string;
  } = {},
): string {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value) search.set(key, value);
  }
  const query = search.toString();
  return `/${encodeSegment(repo)}/log${query ? `?${query}` : ""}`;
}

/** Builds a `/{repo}/commit/{sha}` href, optionally carrying the current diff
 *  display options forward — so following a parent-commit link keeps the
 *  same context/ignore-whitespace/view choice instead of silently resetting
 *  it, and `DiffOptionsBar`'s view pill on the commit page can build its own
 *  "same commit, different view" href through this same function. `path`
 *  restricts the commit's diff to one file — used by the stat view's rows to
 *  link into a single-file diff (there's no file list on that page to jump
 *  an anchor to). */
export function commitHref(repo: string, sha: string, options: Partial<DiffOptions> & { path?: string } = {}): string {
  const search = new URLSearchParams(
    diffOptionsQuery({
      view: options.view ?? "unified",
      context: options.context ?? DEFAULT_CONTEXT,
      ignorews: options.ignorews ?? false,
    }),
  );
  if (options.path) search.set("path", options.path);
  const query = search.toString();
  return `/${encodeSegment(repo)}/commit/${encodeSegment(sha)}${query ? `?${query}` : ""}`;
}

/** Builds a `/{repo}/diff` href from the given comparison + display options,
 *  omitting anything unset/default — same "plain query-string builder"
 *  shape as `searchHref`/`statsHref`, plus `diffOptionsQuery` for the
 *  `view`/`context`/`ignorews` trio. */
export function compareHref(
  repo: string,
  params: { from?: string; to?: string; path?: string } & Partial<DiffOptions> = {},
): string {
  const search = new URLSearchParams();
  if (params.from) search.set("from", params.from);
  if (params.to) search.set("to", params.to);
  if (params.path) search.set("path", params.path);
  const optionsQuery = diffOptionsQuery({
    view: params.view ?? "unified",
    context: params.context ?? DEFAULT_CONTEXT,
    ignorews: params.ignorews ?? false,
  });
  for (const [key, value] of Object.entries(optionsQuery)) search.set(key, value);
  const query = search.toString();
  return `/${encodeSegment(repo)}/diff${query ? `?${query}` : ""}`;
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
