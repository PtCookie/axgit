/** Decodes one path segment, falling back to the raw value for a malformed
 *  escape (a hand-typed URL) rather than throwing. */
function decodeSegment(segment: string): string {
  try {
    return decodeURIComponent(segment);
  } catch {
    return segment;
  }
}

/**
 * The `{repo}` segment of a page URL, decoded. Used by data islands mounted
 * on a `/{repo}/…` shell (built under a placeholder param, see
 * `lib/shell.ts`) to recover the real repository name from `location`.
 */
export function repoFromPathname(pathname: string): string {
  const [segment] = pathname.split("/").filter((part) => part.length > 0);
  if (segment === undefined) return "";
  return decodeSegment(segment);
}

/**
 * The `{sha}` segment of a `/{repo}/commit/{sha}` page URL, decoded. Used by
 * `CommitView`, mounted on the placeholder `/{repo}/commit` shell — the real
 * sha only exists in `location`, never as a build-time prop.
 */
export function commitShaFromPathname(pathname: string): string {
  const [, , sha] = pathname.split("/").filter((part) => part.length > 0);
  if (sha === undefined) return "";
  return decodeSegment(sha);
}

/**
 * The `{path...}` remainder of a `/{repo}/(tree|blob)/{path...}` page URL,
 * decoded segment-by-segment and rejoined with `/`. Used by `TreeView`/
 * `BlobView`, mounted on the placeholder `/{repo}/tree` or `/{repo}/blob`
 * shell — the real path only exists in `location`, never as a build-time
 * prop. Empty for the tree root (`/{repo}/tree`).
 */
export function filePathFromPathname(pathname: string): string {
  const segments = pathname
    .split("/")
    .filter((part) => part.length > 0)
    .slice(2);
  return segments.map(decodeSegment).join("/");
}

/**
 * One query-string param from a page URL (`?ref=`, `?cursor=`, …). Data
 * islands are deliberately not persisted across client-side navigations
 * (`layouts/Layout.astro`, DECISIONS.md #24) — each one remounts fresh on
 * every page and reads `location.search` directly at that point, rather than
 * holding any navigation state of its own.
 */
export function paramFromSearch(name: string, search: string): string | undefined {
  return new URLSearchParams(search).get(name) ?? undefined;
}
