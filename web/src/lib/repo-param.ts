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
